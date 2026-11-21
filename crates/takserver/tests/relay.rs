use std::time::Duration;

use cot::chat::GeoChat;
use cot::framing::{self, Frame, StreamDecoder};
use cot::{Contact, Event, Point, takp};
use futures_util::{SinkExt, StreamExt};
use takserver::{Config, Server};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

async fn server() -> (Server, CancellationToken) {
    let cancel = CancellationToken::new();
    let config = Config {
        tcp: "127.0.0.1:0".parse().unwrap(),
        http: "127.0.0.1:0".parse().unwrap(),
        web_root: None,
        mesh: None,
    };
    (takserver::start(config, cancel.clone()).await.unwrap(), cancel)
}

struct TakClient {
    stream: TcpStream,
    decoder: StreamDecoder,
}

impl TakClient {
    /// Connects and swallows the server's protocol offer, which always comes first.
    async fn connect(server: &Server) -> Self {
        let mut c = Self::connect_raw(server).await;
        let offer = c.next_frame().await.decode().unwrap().event.unwrap();
        assert_eq!(offer.event_type, takp::SUPPORT_TYPE);
        c
    }

    async fn connect_raw(server: &Server) -> Self {
        Self { stream: TcpStream::connect(server.tcp_addr).await.unwrap(), decoder: StreamDecoder::default() }
    }

    async fn send(&mut self, ev: &Event) {
        self.stream.write_all(ev.to_xml().as_bytes()).await.unwrap();
    }

    async fn next_frame(&mut self) -> Frame {
        let mut buf = [0u8; 8192];
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(frame) = self.decoder.next_frame().unwrap() {
                    return frame;
                }
                let n = self.stream.read(&mut buf).await.unwrap();
                assert!(n > 0, "server closed connection");
                self.decoder.extend(&buf[..n]);
            }
        })
        .await
        .expect("timed out waiting for event")
    }

    async fn next(&mut self) -> Event {
        self.next_frame().await.decode().unwrap().event.unwrap()
    }

    async fn assert_silent(&mut self) {
        let mut buf = [0u8; 8192];
        let r = tokio::time::timeout(Duration::from_millis(300), self.stream.read(&mut buf)).await;
        assert!(
            r.is_err(),
            "expected no traffic, got {:?}",
            r.map(|n| String::from_utf8_lossy(&buf[..n.unwrap()]).into_owned())
        );
    }
}

fn sa(uid: &str, callsign: &str) -> Event {
    let mut ev = Event::new(uid, "a-f-G-U-C", Point::new(38.9, -77.0));
    ev.detail.contact = Some(Contact { callsign: callsign.into(), endpoint: Some("*:-1:stcp".into()) });
    ev
}

#[tokio::test]
async fn relays_between_tak_clients_and_browser() {
    let (server, cancel) = server().await;
    let mut alpha = TakClient::connect(&server).await;
    let mut bravo = TakClient::connect(&server).await;
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://{}/ws", server.http_addr)).await.unwrap();

    // TCP client to TCP client and browser; never echoed to the sender.
    alpha.send(&sa("ANDROID-a", "ALPHA")).await;
    assert_eq!(bravo.next().await.uid, "ANDROID-a");
    let msg: serde_json::Value =
        serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(msg["event"]["callsign"], "ALPHA");
    assert_eq!(msg["event"]["sidc"], "SFGPUC---------");
    alpha.assert_silent().await;

    // Browser marker reaches TCP clients as CoT.
    for op in [
        r#"{"op":"hello","uid":"WEB-1","callsign":"KESTREL","team":"Blue","role":"HQ"}"#,
        r#"{"op":"marker","uid":"mk-1","type":"a-h-G","name":"Hostile 1","lat":38.91,"lon":-77.01}"#,
    ] {
        ws.send(Message::Text(op.into())).await.unwrap();
    }
    let marker = bravo.next().await;
    assert_eq!((marker.uid.as_str(), marker.event_type.as_str()), ("mk-1", "a-h-G"));
    assert_eq!(alpha.next().await.uid, "mk-1");

    // Bad ops get an error reply rather than a dropped connection.
    ws.send(Message::Text(r#"{"op":"chat","id":"x","text":""}"#.into())).await.unwrap();
    let reply = loop {
        let v: serde_json::Value =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        if v["kind"] == "error" {
            break v;
        }
    };
    assert!(reply["message"].as_str().unwrap().contains("chat text"));

    // Pings are answered to the sender only.
    let mut ping = Event::new("ANDROID-a-ping", "t-x-c-t", Point::new(0.0, 0.0));
    ping.how = "m-g".into();
    alpha.send(&ping).await;
    assert_eq!(alpha.next().await.event_type, "t-x-c-t-r");
    bravo.assert_silent().await;

    // A late joiner gets the current picture.
    let mut charlie = TakClient::connect(&server).await;
    let mut seen = vec![charlie.next().await.uid, charlie.next().await.uid];
    seen.sort();
    assert_eq!(seen, ["ANDROID-a", "mk-1"]);

    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(5), server.wait()).await.expect("shutdown hung");
}

#[tokio::test]
async fn drops_garbage_stream_but_keeps_serving() {
    let (server, cancel) = server().await;
    let mut junk = TcpStream::connect(server.tcp_addr).await.unwrap();
    junk.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
    let mut rest = Vec::new();
    let closed = tokio::time::timeout(Duration::from_secs(5), junk.read_to_end(&mut rest)).await;
    assert!(closed.is_ok(), "server should close a non-CoT stream");

    let mut a = TakClient::connect(&server).await;
    let mut b = TakClient::connect(&server).await;
    a.send(&sa("u1", "ONE")).await;
    assert_eq!(b.next().await.uid, "u1");
    cancel.cancel();
}

#[tokio::test]
async fn negotiates_protobuf_per_connection() {
    let (server, cancel) = server().await;
    let mut proto = TakClient::connect_raw(&server).await;
    let mut xml = TakClient::connect(&server).await;

    let offer = proto.next_frame().await.decode().unwrap().event.unwrap();
    assert_eq!(takp::supported_versions(&offer), [1]);
    proto.send(&takp::request(&offer.uid, 1, chrono::Utc::now())).await;
    let reply = proto.next_frame().await;
    assert!(matches!(reply, Frame::Xml(_)), "the response itself is still XML");
    let reply = reply.decode().unwrap().event.unwrap();
    assert_eq!((reply.uid.as_str(), takp::response_status(&reply)), (offer.uid.as_str(), Some(true)));

    // From here the negotiated client gets protobuf, the other stays on XML.
    xml.send(&sa("ANDROID-x", "XRAY")).await;
    let frame = proto.next_frame().await;
    assert!(matches!(frame, Frame::Protobuf(_)), "got {frame:?}");
    assert_eq!(frame.decode().unwrap().event.unwrap().detail.contact.unwrap().callsign, "XRAY");

    let mut pb = sa("ANDROID-p", "PAPA");
    pb.detail.xml = "<custom a=\"1\"/>".into();
    proto.stream.write_all(&framing::encode_stream(&pb.to_tak_message())).await.unwrap();
    assert!(matches!(xml.next_frame().await, Frame::Xml(_)));

    // A second request on an already switched stream is refused, not renegotiated.
    proto
        .stream
        .write_all(&framing::encode_stream(&takp::request("again", 1, chrono::Utc::now()).to_tak_message()))
        .await
        .unwrap();
    let again = proto.next_frame().await.decode().unwrap().event.unwrap();
    assert_eq!(takp::response_status(&again), Some(false));
    cancel.cancel();
}

#[tokio::test]
async fn direct_chat_reaches_only_the_recipient() {
    let (server, cancel) = server().await;
    let mut alpha = TakClient::connect(&server).await;
    let mut bravo = TakClient::connect(&server).await;
    let mut charlie = TakClient::connect(&server).await;
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://{}/ws", server.http_addr)).await.unwrap();
    ws.send(Message::Text(
        r#"{"op":"hello","uid":"WEB-1","callsign":"KESTREL","team":"Blue","role":"HQ"}"#.into(),
    ))
    .await
    .unwrap();

    // Routing needs each client's callsign, which the server learns from its SA.
    alpha.send(&sa("ANDROID-a", "ALPHA")).await;
    bravo.send(&sa("ANDROID-b", "BRAVO")).await;
    charlie.send(&sa("ANDROID-c", "CHARLIE")).await;
    for (client, n) in [(&mut alpha, 2), (&mut bravo, 2), (&mut charlie, 2)] {
        for _ in 0..n {
            client.next().await;
        }
    }

    ws.send(Message::Text(
        r#"{"op":"chat","id":"m1","text":"eyes only","to":{"uid":"ANDROID-b","callsign":"BRAVO"}}"#.into(),
    ))
    .await
    .unwrap();
    let got = GeoChat::from_event(&bravo.next().await).unwrap();
    assert_eq!((got.text.as_str(), got.room_id.as_str()), ("eyes only", "ANDROID-b"));
    alpha.assert_silent().await;
    charlie.assert_silent().await;

    // ATAK-style reply, addressed by callsign, reaches the browser and nobody else.
    let reply = GeoChat::direct("ANDROID-b", "BRAVO", "WEB-1", "KESTREL", "m2", "copy", chrono::Utc::now());
    bravo.send(&reply.to_event(Point::new(38.9, -77.0))).await;
    let text = loop {
        let v: serde_json::Value =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        if let Some(text) = v["event"]["chat"]["text"].as_str().filter(|t| *t == "copy") {
            break text.to_owned();
        }
    };
    assert_eq!(text, "copy");
    alpha.assert_silent().await;

    // A browser joining as someone else doesn't get the conversation replayed.
    let (mut other, _) =
        tokio_tungstenite::connect_async(format!("ws://{}/ws", server.http_addr)).await.unwrap();
    other
        .send(Message::Text(
            r#"{"op":"hello","uid":"WEB-2","callsign":"OSPREY","team":"Red","role":"HQ"}"#.into(),
        ))
        .await
        .unwrap();
    let mut chats = 0;
    while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_millis(300), other.next()).await {
        let v: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        chats += usize::from(!v["event"]["chat"].is_null());
    }
    assert_eq!(chats, 0);
    cancel.cancel();
}
