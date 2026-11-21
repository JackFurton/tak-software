use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{DateTime, Duration, Utc};
use cot::marti::{self, Dest};
use cot::{Event, framing};
use roxmltree::Document;
use tokio::sync::broadcast;
use tracing::warn;

use crate::wire;

pub const DELETE_TYPE: &str = "t-x-d-d";
pub const PING_TYPE: &str = "t-x-c-t";

// Bounds memory if a client floods unique uids; real deployments see a few thousand.
const MAX_TRACKED: usize = 50_000;
const QUEUE: usize = 4096;
/// How long a stale event is kept so newly connected UIs can still draw it faded.
const STALE_GRACE: Duration = Duration::minutes(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Mesh,
    Tcp(u64),
    Web(u64),
}

/// Who a connection speaks for, learned from its self SA (TCP) or hello (web).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub uid: String,
    pub callsign: String,
}

/// An event plus its serialized forms, built once and shared by every subscriber.
#[derive(Debug)]
pub struct Envelope {
    pub source: Source,
    pub event: Event,
    pub xml: String,
    pub json: String,
    /// `<marti>` recipients; empty for a broadcast.
    pub dest: Vec<Dest>,
    // Most clients stay on XML, so only build this once someone negotiates protobuf.
    protobuf: OnceLock<Vec<u8>>,
}

impl Envelope {
    pub fn new(source: Source, event: Event) -> Self {
        Self {
            source,
            json: wire::event_json(&event, source),
            xml: event.to_xml(),
            dest: marti::destinations(&event),
            event,
            protobuf: OnceLock::new(),
        }
    }

    /// Stream framing for a TCP client, in whichever encoding it negotiated.
    pub fn stream_bytes(&self, protobuf: bool) -> &[u8] {
        if protobuf {
            self.protobuf.get_or_init(|| framing::encode_stream(&self.event.to_tak_message()))
        } else {
            self.xml.as_bytes()
        }
    }

    /// Whether `peer` should receive this. A connection that hasn't identified itself yet only
    /// gets broadcasts.
    pub fn visible_to(&self, peer: Option<&Peer>) -> bool {
        self.dest.is_empty() || peer.is_some_and(|p| self.dest.iter().any(|d| d.matches(&p.uid, &p.callsign)))
    }
}

/// Fans events out to every connected client and remembers the latest event per uid.
pub struct Hub {
    tx: broadcast::Sender<Arc<Envelope>>,
    latest: Mutex<HashMap<String, Arc<Envelope>>>,
}

impl Default for Hub {
    fn default() -> Self {
        Self { tx: broadcast::channel(QUEUE).0, latest: Mutex::default() }
    }
}

impl Hub {
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Envelope>> {
        self.tx.subscribe()
    }

    pub fn publish(&self, source: Source, event: Event) {
        let env = Arc::new(Envelope::new(source, event));
        {
            let mut latest = self.latest.lock().unwrap();
            if env.event.event_type == DELETE_TYPE {
                for uid in link_uids(&env.event) {
                    latest.remove(&uid);
                }
            } else {
                match latest.get(&env.event.uid) {
                    // Mesh plus a server path delivers the same event twice, and UDP reorders.
                    Some(prev) if prev.event.time >= env.event.time => return,
                    None if latest.len() >= MAX_TRACKED => {
                        warn!(uid = %env.event.uid, "tracking limit reached, relaying without storing");
                    }
                    _ => {
                        latest.insert(env.event.uid.clone(), env.clone());
                    }
                }
            }
        }
        // No subscribers is not an error.
        let _ = self.tx.send(env);
    }

    /// Everything currently tracked, oldest first so replay order matches arrival order.
    pub fn snapshot(&self) -> Vec<Arc<Envelope>> {
        let mut all: Vec<_> = self.latest.lock().unwrap().values().cloned().collect();
        all.sort_by_key(|e| e.event.time);
        all
    }

    pub fn purge(&self, now: DateTime<Utc>) {
        self.latest.lock().unwrap().retain(|_, e| e.event.stale + STALE_GRACE > now);
    }

    pub fn tracked(&self) -> usize {
        self.latest.lock().unwrap().len()
    }
}

/// Targets of `<link uid=...>` elements; a delete event names what it deletes this way.
pub fn link_uids(ev: &Event) -> Vec<String> {
    let wrapped = format!("<detail>{}</detail>", ev.detail.xml);
    let Ok(doc) = Document::parse(&wrapped) else { return Vec::new() };
    doc.root_element()
        .children()
        .filter(|n| n.has_tag_name("link"))
        .filter_map(|n| n.attribute("uid").map(String::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cot::Point;

    fn ev(uid: &str, at: DateTime<Utc>) -> Event {
        let mut e = Event::new(uid, "a-f-G", Point::new(0.0, 0.0));
        e.time = at;
        e.stale = at + Duration::minutes(1);
        e
    }

    #[test]
    fn keeps_newest_per_uid() {
        let hub = Hub::default();
        let t = Utc::now();
        hub.publish(Source::Mesh, ev("a", t));
        hub.publish(Source::Mesh, ev("a", t - Duration::seconds(5)));
        hub.publish(Source::Mesh, ev("b", t));
        let snap = hub.snapshot();
        assert_eq!(snap.len(), 2);
        assert!(snap.iter().all(|e| e.event.time == t));
    }

    #[test]
    fn delete_removes_target() {
        let hub = Hub::default();
        hub.publish(Source::Web(1), ev("marker-1", Utc::now()));
        let mut del = ev("d", Utc::now());
        del.event_type = DELETE_TYPE.into();
        del.detail.xml = r#"<link uid="marker-1" relation="none" type="a-h-G"/><__forcedelete/>"#.into();
        let mut rx = hub.subscribe();
        hub.publish(Source::Web(1), del);
        assert_eq!(hub.tracked(), 0);
        assert_eq!(rx.try_recv().unwrap().event.event_type, DELETE_TYPE);
    }

    #[test]
    fn directed_only_to_recipient() {
        let mut e = ev("chat", Utc::now());
        e.detail.xml = marti::to_callsigns(["COBRA"]);
        let env = Envelope::new(Source::Web(1), e);
        let peer = |uid: &str, callsign: &str| Peer { uid: uid.into(), callsign: callsign.into() };
        assert!(env.visible_to(Some(&peer("ANDROID-2", "COBRA"))));
        assert!(!env.visible_to(Some(&peer("ANDROID-3", "VIPER"))));
        assert!(!env.visible_to(None));
        assert!(Envelope::new(Source::Mesh, ev("sa", Utc::now())).visible_to(None));
    }

    #[test]
    fn purges_after_grace() {
        let hub = Hub::default();
        let t = Utc::now();
        hub.publish(Source::Mesh, ev("a", t));
        hub.purge(t + Duration::minutes(5));
        assert_eq!(hub.tracked(), 1);
        hub.purge(t + Duration::minutes(12));
        assert_eq!(hub.tracked(), 0);
    }
}
