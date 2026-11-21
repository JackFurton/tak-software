use std::fmt::Write;

use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use roxmltree::{Document, Node, ParsingOptions};

use crate::{Contact, Detail, Error, Event, Group, Point, PrecisionLocation, Result, Status, Takv, Track};

// Well above any real event; bounds memory spent on hostile input.
const NODES_LIMIT: u32 = 10_000;

fn parse_document(text: &str) -> std::result::Result<Document<'_>, roxmltree::Error> {
    let opts = ParsingOptions { allow_dtd: false, nodes_limit: NODES_LIMIT, ..Default::default() };
    Document::parse_with_options(text, opts)
}

impl Event {
    pub fn from_xml(text: &str) -> Result<Self> {
        let doc = parse_document(text)?;
        let root = doc.root_element();
        if root.tag_name().name() != "event" {
            return Err(Error::NotAnEvent(root.tag_name().name().into()));
        }

        let point =
            root.children().find(|n| n.has_tag_name("point")).ok_or(Error::MissingElement("point"))?;
        let point = Point {
            lat: parse_attr(point, "lat")?,
            lon: parse_attr(point, "lon")?,
            hae: parse_attr_or(point, "hae", Point::UNKNOWN)?,
            ce: parse_attr_or(point, "ce", Point::UNKNOWN)?,
            le: parse_attr_or(point, "le", Point::UNKNOWN)?,
        };
        point.validate()?;

        let detail = match root.children().find(|n| n.has_tag_name("detail")) {
            Some(node) => Detail::from_node(node, text),
            None => Detail::default(),
        };

        let opt = |name| root.attribute(name).map(String::from);
        Ok(Self {
            uid: require_attr(root, "uid")?.into(),
            event_type: require_attr(root, "type")?.into(),
            how: root.attribute("how").unwrap_or_default().into(),
            time: parse_time(root, "time")?,
            start: parse_time(root, "start")?,
            stale: parse_time(root, "stale")?,
            access: opt("access"),
            qos: opt("qos"),
            opex: opt("opex"),
            point,
            detail,
        })
    }

    pub fn to_xml(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
        out.push_str(r#"<event version="2.0""#);
        attr(&mut out, "uid", &self.uid);
        attr(&mut out, "type", &self.event_type);
        if !self.how.is_empty() {
            attr(&mut out, "how", &self.how);
        }
        attr(&mut out, "time", &format_time(self.time));
        attr(&mut out, "start", &format_time(self.start));
        attr(&mut out, "stale", &format_time(self.stale));
        for (name, value) in [("access", &self.access), ("qos", &self.qos), ("opex", &self.opex)] {
            if let Some(v) = value {
                attr(&mut out, name, v);
            }
        }
        out.push('>');

        let p = &self.point;
        let _ = write!(
            out,
            r#"<point lat="{}" lon="{}" hae="{}" ce="{}" le="{}"/>"#,
            p.lat, p.lon, p.hae, p.ce, p.le
        );
        self.detail.write_xml(&mut out);
        out.push_str("</event>");
        out
    }
}

impl Detail {
    fn from_node(node: Node, text: &str) -> Self {
        let mut detail = Detail::default();
        // Per detail.proto, an element that repeats is never promoted: every copy stays raw.
        let unique = |name: &str| node.children().filter(|c| c.has_tag_name(name)).nth(1).is_none();
        for child in node.children().filter(Node::is_element) {
            let name = child.tag_name().name();
            let taken = match name {
                "contact" if unique(name) => typed(child, &["callsign"], &["endpoint"], |a| Contact {
                    callsign: a.req("callsign"),
                    endpoint: a.opt("endpoint"),
                })
                .map(|v| detail.contact = Some(v)),
                "__group" if unique(name) => typed(child, &["name", "role"], &[], |a| Group {
                    name: a.req("name"),
                    role: a.req("role"),
                })
                .map(|v| detail.group = Some(v)),
                "precisionlocation" if unique(name) => typed(child, &[], &["geopointsrc", "altsrc"], |a| {
                    PrecisionLocation { geopointsrc: a.opt("geopointsrc"), altsrc: a.opt("altsrc") }
                })
                .map(|v| detail.precision_location = Some(v)),
                "status" if unique(name) => {
                    typed(child, &["battery"], &[], |a| a.req("battery").parse().ok())
                        .flatten()
                        .map(|battery| detail.status = Some(Status { battery }))
                }
                "takv" if unique(name) => {
                    typed(child, &[], &["device", "platform", "os", "version"], |a| Takv {
                        device: a.opt("device"),
                        platform: a.opt("platform"),
                        os: a.opt("os"),
                        version: a.opt("version"),
                    })
                    .map(|v| detail.takv = Some(v))
                }
                "track" if unique(name) => typed(child, &["speed", "course"], &[], |a| {
                    let speed = a.req("speed").parse().ok().filter(|v: &f64| v.is_finite())?;
                    let course = a.req("course").parse().ok().filter(|v: &f64| v.is_finite())?;
                    Some(Track { speed, course })
                })
                .flatten()
                .map(|v| detail.track = Some(v)),
                _ => None,
            };
            if taken.is_none() {
                detail.xml.push_str(&text[child.range()]);
            }
        }
        detail
    }

    fn write_xml(&self, out: &mut String) {
        if self.is_empty() {
            return;
        }
        out.push_str("<detail>");
        if let Some(c) = &self.contact {
            element(out, "contact", &[("callsign", Some(&c.callsign)), ("endpoint", c.endpoint.as_ref())]);
        }
        if let Some(g) = &self.group {
            element(out, "__group", &[("name", Some(&g.name)), ("role", Some(&g.role))]);
        }
        if let Some(p) = &self.precision_location {
            element(
                out,
                "precisionlocation",
                &[("geopointsrc", p.geopointsrc.as_ref()), ("altsrc", p.altsrc.as_ref())],
            );
        }
        if let Some(s) = &self.status {
            element(out, "status", &[("battery", Some(&s.battery.to_string()))]);
        }
        if let Some(t) = &self.takv {
            element(
                out,
                "takv",
                &[
                    ("device", t.device.as_ref()),
                    ("platform", t.platform.as_ref()),
                    ("os", t.os.as_ref()),
                    ("version", t.version.as_ref()),
                ],
            );
        }
        if let Some(t) = &self.track {
            element(
                out,
                "track",
                &[("speed", Some(&t.speed.to_string())), ("course", Some(&t.course.to_string()))],
            );
        }
        out.push_str(&self.xml);
        out.push_str("</detail>");
    }

    /// Accepts raw detail XML from an untrusted source (a protobuf `xmlDetail`). It is spliced
    /// verbatim into XML output, so anything unbalanced could close `<event>` early and inject a
    /// second event into a legacy XML stream.
    pub(crate) fn validate_raw(xml: &str) -> Result<()> {
        if xml.is_empty() {
            return Ok(());
        }
        parse_document(&format!("<detail>{xml}</detail>")).map(drop).map_err(Error::InvalidXmlDetail)
    }
}

struct Attrs<'a, 'input>(Node<'a, 'input>);

impl Attrs<'_, '_> {
    fn req(&self, name: &str) -> String {
        self.0.attribute(name).unwrap_or_default().into()
    }

    fn opt(&self, name: &str) -> Option<String> {
        self.0.attribute(name).map(String::from)
    }
}

/// Builds a typed detail element only if doing so loses nothing: no child content, no attributes
/// beyond `required` and `optional`, every `required` present, and no empty optional (protobuf
/// can't tell empty from absent). Anything else stays raw.
fn typed<T>(node: Node, required: &[&str], optional: &[&str], build: impl FnOnce(Attrs) -> T) -> Option<T> {
    let has_content =
        node.children().any(|c| c.is_element() || c.text().is_some_and(|t| !t.trim().is_empty()));
    let all_known = node
        .attributes()
        .all(|a| required.contains(&a.name()) || (optional.contains(&a.name()) && !a.value().is_empty()));
    let has_required = required.iter().all(|r| node.attribute(*r).is_some());
    (!has_content && all_known && has_required && node.tag_name().namespace().is_none())
        .then(|| build(Attrs(node)))
}

fn require_attr<'a>(node: Node<'a, '_>, name: &'static str) -> Result<&'a str> {
    node.attribute(name).ok_or(Error::MissingAttribute(name))
}

fn parse_attr(node: Node, name: &'static str) -> Result<f64> {
    let raw = require_attr(node, name)?;
    raw.trim().parse().map_err(|_| Error::InvalidValue { name, value: raw.into() })
}

fn parse_attr_or(node: Node, name: &'static str, default: f64) -> Result<f64> {
    match node.attribute(name) {
        Some(_) => parse_attr(node, name),
        None => Ok(default),
    }
}

fn parse_time(node: Node, name: &'static str) -> Result<DateTime<Utc>> {
    let raw = require_attr(node, name)?;
    DateTime::parse_from_rfc3339(raw)
        .map(|t| t.with_timezone(&Utc))
        // Some non-ATAK senders drop the trailing Z; CoT times are always UTC.
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f").map(|t| t.and_utc()))
        .map_err(|_| Error::InvalidValue { name, value: raw.into() })
}

fn format_time(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn element(out: &mut String, name: &str, attrs: &[(&str, Option<&String>)]) {
    out.push('<');
    out.push_str(name);
    for (k, v) in attrs {
        if let Some(v) = v {
            attr(out, k, v);
        }
    }
    out.push_str("/>");
}

fn attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    escape_into(out, value);
    out.push('"');
}

/// Escapes text for use in an XML attribute value or element content.
pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    escape_into(&mut out, value);
    out
}

fn escape_into(out: &mut String, value: &str) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            // XML 1.0 forbids other control characters even as character references.
            c if (c as u32) < 0x20 => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
}
