//! GeoChat, ATAK's text chat. A message is a `b-t-f` event whose detail carries the room in
//! `<__chat>` and the text in `<remarks>`.

use chrono::{DateTime, Duration, Utc};
use roxmltree::{Document, Node};

use crate::{Event, Point, xml_escape};

pub const CHAT_TYPE: &str = "b-t-f";
pub const ALL_CHAT_ROOMS: &str = "All Chat Rooms";

#[derive(Debug, Clone, PartialEq)]
pub struct GeoChat {
    pub message_id: String,
    pub sender_uid: String,
    pub sender_callsign: String,
    /// Display name of the room; for direct messages ATAK uses the recipient's callsign.
    pub room: String,
    /// Room identifier; equals `room` for All Chat Rooms, the recipient's uid for direct messages.
    pub room_id: String,
    pub text: String,
    pub time: DateTime<Utc>,
}

impl GeoChat {
    pub fn to_all(
        sender_uid: impl Into<String>,
        sender_callsign: impl Into<String>,
        message_id: impl Into<String>,
        text: impl Into<String>,
        time: DateTime<Utc>,
    ) -> Self {
        Self {
            message_id: message_id.into(),
            sender_uid: sender_uid.into(),
            sender_callsign: sender_callsign.into(),
            room: ALL_CHAT_ROOMS.into(),
            room_id: ALL_CHAT_ROOMS.into(),
            text: text.into(),
            time,
        }
    }

    /// `point` is the sender's position; ATAK plots chat origins on the map.
    pub fn to_event(&self, point: Point) -> Event {
        let mut ev = Event::new(
            format!("GeoChat.{}.{}.{}", self.sender_uid, self.room_id, self.message_id),
            CHAT_TYPE,
            point,
        );
        ev.how = "h-g-i-g-o".into();
        ev.time = self.time;
        ev.start = self.time;
        ev.stale = self.time + Duration::days(1);

        let e = xml_escape;
        let time = self.time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        ev.detail.xml = format!(
            concat!(
                r#"<__chat parent="RootContactGroup" groupOwner="false" messageId="{msg}" "#,
                r#"chatroom="{room}" id="{room_id}" senderCallsign="{callsign}">"#,
                r#"<chatgrp uid0="{uid}" uid1="{room_id}" id="{room_id}"/></__chat>"#,
                r#"<link uid="{uid}" type="a-f-G-U-C" relation="p-p"/>"#,
                r#"<remarks source="BAO.F.ATAK.{uid}" to="{room_id}" time="{time}">{text}</remarks>"#,
            ),
            msg = e(&self.message_id),
            room = e(&self.room),
            room_id = e(&self.room_id),
            callsign = e(&self.sender_callsign),
            uid = e(&self.sender_uid),
            time = time,
            text = e(&self.text),
        );
        ev
    }

    /// `None` if `ev` isn't a chat message or lacks the elements every ATAK version sends.
    pub fn from_event(ev: &Event) -> Option<Self> {
        if ev.event_type != CHAT_TYPE {
            return None;
        }
        let wrapped = format!("<detail>{}</detail>", ev.detail.xml);
        let doc = Document::parse(&wrapped).ok()?;
        let detail = doc.root_element();
        let child = |name| detail.children().find(|n: &Node| n.has_tag_name(name));

        let chat = child("__chat")?;
        let remarks = child("remarks")?;
        let sender_uid = chat
            .children()
            .find(|n| n.has_tag_name("chatgrp"))
            .and_then(|g| g.attribute("uid0"))
            .or_else(|| child("link").and_then(|l| l.attribute("uid")))?;
        let room = chat.attribute("chatroom")?;
        let time = remarks
            .attribute("time")
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map_or(ev.time, |t| t.with_timezone(&Utc));

        Some(Self {
            message_id: chat.attribute("messageId").unwrap_or(&ev.uid).into(),
            sender_uid: sender_uid.into(),
            sender_callsign: chat.attribute("senderCallsign").unwrap_or(sender_uid).into(),
            room: room.into(),
            room_id: chat.attribute("id").unwrap_or(room).into(),
            text: remarks.text().unwrap_or_default().into(),
            time,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Detail as sent by ATAK-CIV to All Chat Rooms.
    const ATAK_CHAT: &str = r#"<event version="2.0" uid="GeoChat.ANDROID-1.All Chat Rooms.9f2c" type="b-t-f"
        how="h-g-i-g-o" time="2026-11-20T15:00:00.000Z" start="2026-11-20T15:00:00.000Z"
        stale="2026-11-21T15:00:00.000Z"><point lat="38.9" lon="-77.0" hae="9999999.0" ce="9999999.0"
        le="9999999.0"/><detail><__chat parent="RootContactGroup" groupOwner="false" messageId="9f2c"
        chatroom="All Chat Rooms" id="All Chat Rooms" senderCallsign="VIPER"><chatgrp uid0="ANDROID-1"
        uid1="All Chat Rooms" id="All Chat Rooms"/></__chat><link uid="ANDROID-1" type="a-f-G-U-C"
        relation="p-p"/><remarks source="BAO.F.ATAK.ANDROID-1" to="All Chat Rooms"
        time="2026-11-20T15:00:00.000Z">moving to rally point &amp; holding</remarks></detail></event>"#;

    #[test]
    fn parses_atak_chat() {
        let chat = GeoChat::from_event(&Event::from_xml(ATAK_CHAT).unwrap()).unwrap();
        assert_eq!(chat.sender_uid, "ANDROID-1");
        assert_eq!(chat.sender_callsign, "VIPER");
        assert_eq!(chat.room, ALL_CHAT_ROOMS);
        assert_eq!(chat.message_id, "9f2c");
        assert_eq!(chat.text, "moving to rally point & holding");
    }

    #[test]
    fn round_trips_through_xml() {
        let t = DateTime::parse_from_rfc3339("2026-11-20T15:00:00.250Z").unwrap().with_timezone(&Utc);
        let chat = GeoChat::to_all("WEB-1", "R&D <ops>", "m-1", "a < b & \"c\"", t);
        let ev = chat.to_event(Point::new(1.0, 2.0));
        assert_eq!(ev.uid, "GeoChat.WEB-1.All Chat Rooms.m-1");
        let back = Event::from_xml(&ev.to_xml()).unwrap();
        assert_eq!(GeoChat::from_event(&back), Some(chat));
    }

    #[test]
    fn ignores_other_types() {
        let ev = Event::new("x", "a-f-G", Point::new(0.0, 0.0));
        assert_eq!(GeoChat::from_event(&ev), None);
    }
}
