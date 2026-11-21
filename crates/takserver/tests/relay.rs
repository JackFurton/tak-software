use std::time::Duration;

use cot::framing::StreamDecoder;
use cot::{Contact, Event, Point};
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
    async fn connect(server: &Server) -> Self {
        Self { stream: TcpStream::connect(server.tcp_addr).await.unwrap(), decoder: StreamDecoder::default() }
    }

    async fn send(&mut self, ev: &Event) {
        self.stream.write_all(ev.to_xml().as_bytes()).await.unwrap();
    }

    async fn next(&mut self) -> Event {
        let mut buf = [0u8; 8192];
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(frame) = self.decoder.next_frame().unwrap() {
                    return frame.decode().unwrap().event.unwrap();
                }
                let n = self.stream.read(&mut buf).await.unwrap();
                assert!(n > 0, "server closed connection");
                self.decoder.extend(&buf[..n]);
            }
        })
        .await
        .expect("timed out waiting for event")
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
    let mut buf = [0u8; 16];
    let n = tokio::time::timeout(Duration::from_secs(5), junk.read(&mut buf)).await.unwrap().unwrap_or(0);
    assert_eq!(n, 0, "server should close a non-CoT stream");

    let mut a = TakClient::connect(&server).await;
    let mut b = TakClient::connect(&server).await;
    a.send(&sa("u1", "ONE")).await;
    assert_eq!(b.next().await.uid, "u1");
    cancel.cancel();
}
