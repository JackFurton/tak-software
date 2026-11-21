//! Wire framing for TAK protocol v1 and legacy XML.
//!
//! Mesh (UDP multicast): one message per datagram, either raw XML or `0xbf 0x01 0xbf` followed by
//! a protobuf `TakMessage`. Streaming (TCP/TLS): XML events back to back, or `0xbf`, a varint
//! length, then the protobuf `TakMessage`. A stream starts as XML and switches to protobuf after
//! negotiation, so the decoder accepts either at every frame boundary.

use prost::Message as _;

use crate::proto::{TakControl, TakMessage};
use crate::{Error, Event, Result};

const MAGIC: u8 = 0xbf;
const MESH_HEADER: [u8; 3] = [MAGIC, 0x01, MAGIC];
const XML_END: &[u8] = b"</event>";

/// Default per-frame limit. Real events are a few KB; large payloads travel as data packages
/// over HTTP, not inside CoT.
pub const DEFAULT_MAX_FRAME: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Xml(Vec<u8>),
    Protobuf(Vec<u8>),
}

/// A decoded frame. XML frames never carry `control`.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub control: Option<TakControl>,
    pub event: Option<Event>,
}

impl Frame {
    pub fn decode(&self) -> Result<Message> {
        match self {
            Frame::Xml(bytes) => {
                let text = std::str::from_utf8(bytes).map_err(|_| Error::NotUtf8)?;
                Ok(Message { control: None, event: Some(Event::from_xml(text)?) })
            }
            Frame::Protobuf(bytes) => {
                let msg = TakMessage::decode(bytes.as_slice())?;
                Ok(Message {
                    control: msg.tak_control,
                    event: msg.cot_event.map(Event::from_proto).transpose()?,
                })
            }
        }
    }
}

pub fn encode_mesh(msg: &TakMessage) -> Vec<u8> {
    let mut out = Vec::with_capacity(MESH_HEADER.len() + msg.encoded_len());
    out.extend_from_slice(&MESH_HEADER);
    msg.encode(&mut out).expect("Vec grows on demand");
    out
}

pub fn encode_stream(msg: &TakMessage) -> Vec<u8> {
    let len = msg.encoded_len();
    let mut out = Vec::with_capacity(1 + prost::length_delimiter_len(len) + len);
    out.push(MAGIC);
    msg.encode_length_delimited(&mut out).expect("Vec grows on demand");
    out
}

pub fn decode_datagram(data: &[u8]) -> Result<Frame> {
    match data.first() {
        Some(&MAGIC) => data
            .strip_prefix(&MESH_HEADER)
            .map(|body| Frame::Protobuf(body.to_vec()))
            .ok_or(Error::BadMeshHeader),
        _ => Ok(Frame::Xml(data.to_vec())),
    }
}

/// Splits a TCP byte stream into frames. Framing errors are not recoverable: the stream position
/// is lost, so the caller should drop the connection. Errors from [`Frame::decode`] only affect
/// that one frame.
#[derive(Debug)]
pub struct StreamDecoder {
    buf: Vec<u8>,
    max_frame: usize,
    // Where to resume searching for `</event>`, so a slowly trickling frame isn't rescanned.
    scan_from: usize,
}

impl Default for StreamDecoder {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_FRAME)
    }
}

impl StreamDecoder {
    pub fn new(max_frame: usize) -> Self {
        Self { buf: Vec::new(), max_frame, scan_from: 0 }
    }

    pub fn extend(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Returns the next complete frame, or `None` until more bytes arrive.
    pub fn next_frame(&mut self) -> Result<Option<Frame>> {
        let skip = self.buf.iter().take_while(|b| b.is_ascii_whitespace()).count();
        if skip > 0 {
            self.buf.drain(..skip);
            self.scan_from = self.scan_from.saturating_sub(skip);
        }
        match self.buf.first() {
            None => Ok(None),
            Some(&MAGIC) => self.next_protobuf(),
            Some(b'<') => self.next_xml(),
            Some(&b) => Err(Error::UnexpectedByte(b)),
        }
    }

    fn next_protobuf(&mut self) -> Result<Option<Frame>> {
        let Some((len, header)) = read_varint(&self.buf[1..]) else {
            return Ok(None);
        };
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        if len > self.max_frame {
            return Err(Error::FrameTooLarge { len, limit: self.max_frame });
        }
        let start = 1 + header;
        if self.buf.len() < start + len {
            return Ok(None);
        }
        let body = self.buf[start..start + len].to_vec();
        self.buf.drain(..start + len);
        self.scan_from = 0;
        Ok(Some(Frame::Protobuf(body)))
    }

    // Splits on the first `</event>`, as ATAK does. A `</event>` inside CDATA cuts the frame
    // short; that frame fails to decode and the stream resyncs at the next real end tag.
    fn next_xml(&mut self) -> Result<Option<Frame>> {
        let found = self.buf[self.scan_from..]
            .windows(XML_END.len())
            .position(|w| w == XML_END)
            .map(|i| self.scan_from + i + XML_END.len());
        let Some(end) = found else {
            if self.buf.len() > self.max_frame {
                return Err(Error::FrameTooLarge { len: self.buf.len(), limit: self.max_frame });
            }
            self.scan_from = self.buf.len().saturating_sub(XML_END.len() - 1);
            return Ok(None);
        };
        if end > self.max_frame {
            return Err(Error::FrameTooLarge { len: end, limit: self.max_frame });
        }
        let frame = self.buf.drain(..end).collect();
        self.scan_from = 0;
        Ok(Some(Frame::Xml(frame)))
    }
}

/// Returns the value and the number of bytes it occupied, or `None` if `buf` ends mid-varint.
fn read_varint(buf: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for (i, &b) in buf.iter().enumerate().take(10) {
        value |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    // Ten continuation bytes can't be a valid length; report it as oversized rather than waiting.
    (buf.len() >= 10).then_some((u64::MAX, 10))
}

#[cfg(test)]
mod tests {
    use super::read_varint;

    #[test]
    fn varint() {
        assert_eq!(read_varint(&[0x05]), Some((5, 1)));
        assert_eq!(read_varint(&[0xac, 0x02]), Some((300, 2)));
        assert_eq!(read_varint(&[0xac]), None);
        assert_eq!(read_varint(&[0xff; 10]), Some((u64::MAX, 10)));
    }
}
