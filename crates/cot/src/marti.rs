//! `<marti>` addressing. An event whose detail holds `<marti><dest callsign="X"/></marti>` is
//! meant only for the named recipients; TAK Server delivers it to them instead of everyone.
//! ATAK addresses by callsign; `uid` is accepted too, as TAK Server does.

use roxmltree::Document;

use crate::{Event, xml_escape};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dest {
    pub uid: Option<String>,
    pub callsign: Option<String>,
}

impl Dest {
    pub fn matches(&self, uid: &str, callsign: &str) -> bool {
        self.uid.as_deref() == Some(uid) || self.callsign.as_deref() == Some(callsign)
    }
}

/// Recipients named by `ev`. Empty means a broadcast. Dests naming only a mission are skipped:
/// missions aren't supported, and treating them as nobody would silently swallow the event.
pub fn destinations(ev: &Event) -> Vec<Dest> {
    if !ev.detail.xml.contains("marti") {
        return Vec::new();
    }
    let wrapped = format!("<detail>{}</detail>", ev.detail.xml);
    let Ok(doc) = Document::parse(&wrapped) else { return Vec::new() };
    doc.root_element()
        .children()
        .filter(|n| n.has_tag_name("marti"))
        .flat_map(|m| m.children())
        .filter(|n| n.has_tag_name("dest"))
        .map(|n| Dest {
            uid: n.attribute("uid").map(String::from),
            callsign: n.attribute("callsign").map(String::from),
        })
        .filter(|d| d.uid.is_some() || d.callsign.is_some())
        .collect()
}

pub fn to_callsigns<'a>(callsigns: impl IntoIterator<Item = &'a str>) -> String {
    let dests: String =
        callsigns.into_iter().map(|c| format!(r#"<dest callsign="{}"/>"#, xml_escape(c))).collect();
    format!("<marti>{dests}</marti>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Point;

    fn with_detail(xml: &str) -> Event {
        let mut ev = Event::new("x", "b-t-f", Point::new(0.0, 0.0));
        ev.detail.xml = xml.into();
        ev
    }

    #[test]
    fn reads_dests() {
        let ev = with_detail(
            r#"<remarks>hi</remarks><marti><dest callsign="COBRA"/><dest uid="ANDROID-2"/><dest mission="ops"/></marti>"#,
        );
        let dests = destinations(&ev);
        assert_eq!(dests.len(), 2);
        assert!(dests[0].matches("other", "COBRA"));
        assert!(dests[1].matches("ANDROID-2", "other"));
        assert!(!dests.iter().any(|d| d.matches("ANDROID-3", "VIPER")));
    }

    #[test]
    fn broadcast_has_none() {
        assert!(destinations(&with_detail("<remarks>marti</remarks>")).is_empty());
        assert!(destinations(&with_detail(&to_callsigns([]))).is_empty());
    }

    #[test]
    fn writes_escaped() {
        let ev = with_detail(&to_callsigns(["R&D"]));
        assert_eq!(destinations(&ev)[0].callsign.as_deref(), Some("R&D"));
    }
}
