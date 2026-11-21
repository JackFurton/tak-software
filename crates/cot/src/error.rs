pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("malformed XML: {0}")]
    Xml(#[from] roxmltree::Error),
    #[error("malformed protobuf: {0}")]
    Protobuf(#[from] prost::DecodeError),
    #[error("root element is <{0}>, expected <event>")]
    NotAnEvent(String),
    #[error("missing <{0}> element")]
    MissingElement(&'static str),
    #[error("missing attribute {0:?}")]
    MissingAttribute(&'static str),
    #[error("invalid {name}: {value:?}")]
    InvalidValue { name: &'static str, value: String },
    #[error("xmlDetail is not well-formed XML content: {0}")]
    InvalidXmlDetail(roxmltree::Error),
    #[error("frame of {len} bytes exceeds limit of {limit}")]
    FrameTooLarge { len: usize, limit: usize },
    #[error("unexpected byte {0:#04x} at start of frame")]
    UnexpectedByte(u8),
    #[error("mesh datagram has no TAK protocol v1 header")]
    BadMeshHeader,
    #[error("XML frame is not valid UTF-8")]
    NotUtf8,
}
