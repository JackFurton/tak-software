//! Cursor on Target (CoT): the message format spoken by ATAK, WinTAK, iTAK and TAK Server.
//!
//! Covers the event model, the legacy XML encoding, TAK protocol v1 (protobuf), and the
//! framing used on UDP mesh and TCP streams.

mod cot_type;
mod error;
mod event;
pub mod framing;
pub mod proto;
mod xml;

pub use cot_type::{Affiliation, CotType, Dimension};
pub use error::{Error, Result};
pub use event::{Contact, Detail, Event, Group, Point, PrecisionLocation, Status, Takv, Track};
