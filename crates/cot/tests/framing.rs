use cot::framing::{Frame, StreamDecoder, decode_datagram, encode_mesh, encode_stream};
use cot::{Error, Event, Point};

fn event(uid: &str) -> Event {
    Event::new(uid, "a-f-G-U-C", Point::new(1.0, 2.0))
}

fn drain(dec: &mut StreamDecoder) -> Vec<Frame> {
    std::iter::from_fn(|| dec.next_frame().unwrap()).collect()
}

#[test]
fn mesh_round_trip() {
    let ev = event("m");
    let bytes = encode_mesh(&ev.to_tak_message());
    assert_eq!(&bytes[..3], &[0xbf, 0x01, 0xbf]);
    let msg = decode_datagram(&bytes).unwrap().decode().unwrap();
    assert_eq!(msg.event, Some(Event::from_proto(ev.to_proto()).unwrap()));
}

#[test]
fn mesh_xml_datagram() {
    let ev = event("x");
    let frame = decode_datagram(ev.to_xml().as_bytes()).unwrap();
    assert!(matches!(frame, Frame::Xml(_)));
    assert_eq!(frame.decode().unwrap().event.unwrap().uid, "x");
}

#[test]
fn mesh_rejects_other_versions() {
    assert!(matches!(decode_datagram(&[0xbf, 0x02, 0xbf]), Err(Error::BadMeshHeader)));
}

#[test]
fn stream_mixed_formats_byte_by_byte() {
    let mut wire = Vec::new();
    wire.extend_from_slice(event("a").to_xml().as_bytes());
    wire.extend_from_slice(b"\n\n");
    wire.extend_from_slice(event("b").to_xml().as_bytes());
    wire.extend_from_slice(&encode_stream(&event("c").to_tak_message()));
    wire.extend_from_slice(&encode_stream(&event("d").to_tak_message()));

    let mut dec = StreamDecoder::default();
    let mut uids = Vec::new();
    for b in &wire {
        dec.extend(std::slice::from_ref(b));
        while let Some(frame) = dec.next_frame().unwrap() {
            uids.push(frame.decode().unwrap().event.unwrap().uid);
        }
    }
    assert_eq!(uids, ["a", "b", "c", "d"]);
}

#[test]
fn stream_bad_frame_is_isolated() {
    let mut dec = StreamDecoder::default();
    dec.extend(b"<event uid=\"broken\"></event>");
    dec.extend(event("ok").to_xml().as_bytes());
    let frames = drain(&mut dec);
    assert_eq!(frames.len(), 2);
    assert!(frames[0].decode().is_err());
    assert_eq!(frames[1].decode().unwrap().event.unwrap().uid, "ok");
}

#[test]
fn stream_limits_frame_size() {
    let mut dec = StreamDecoder::new(64);
    dec.extend(&[b'<'; 65]);
    assert!(matches!(dec.next_frame(), Err(Error::FrameTooLarge { .. })));

    let mut dec = StreamDecoder::new(64);
    dec.extend(&[0xbf, 0x80, 0x01]);
    assert!(matches!(dec.next_frame(), Err(Error::FrameTooLarge { len: 128, .. })));
}

#[test]
fn stream_rejects_garbage() {
    let mut dec = StreamDecoder::default();
    dec.extend(b"GET / HTTP/1.1\r\n");
    assert!(matches!(dec.next_frame(), Err(Error::UnexpectedByte(b'G'))));
}
