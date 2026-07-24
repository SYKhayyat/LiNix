//! LiNix's own events, and the payload a hook on one receives (XIII.13, U15).
//!
//! Every integration request — notify me, push the repo, open a ticket — used to have to become
//! a LiNix feature. An event with a script on it is the general answer, so these three are the
//! seam: `after_sync`, `on_drift`, `on_guard_refusal`.
//!
//! Pure. The enum, the payload, and the JSON. Finding the scripts and running them is
//! `app::events`.

use serde::Serialize;

/// The schema version of the JSON a hook reads on stdin.
///
/// Versioned for U17's reason, one domain over: a hook is a program someone else wrote, LiNix
/// cannot see it, and it will break if a field changes meaning under it.
pub const PAYLOAD_SCHEMA: u32 = 1;

/// An event a hook can be attached to.
///
/// **Closed.** A hook file named after something not in this list is a typo, and a typo that
/// silently never fires is the worst possible outcome for a notification hook — you find out
/// when the thing you wanted to be told about has already happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Event {
    /// A sync finished. Fires whether or not anything changed.
    AfterSync,
    /// A sync found the machine had drifted from the configuration.
    OnDrift,
    /// The guard refused something (II.10). The hook cannot un-refuse it; it is told.
    OnGuardRefusal,
}

impl Event {
    /// Every event, for the "unknown event" diagnostic and for `linix lock`'s sweep.
    pub const ALL: [Event; 3] = [Event::AfterSync, Event::OnDrift, Event::OnGuardRefusal];

    /// The name a user writes: the hook filename, and the `[hooks.<name>]` table key.
    pub fn as_str(self) -> &'static str {
        match self {
            Event::AfterSync => "after_sync",
            Event::OnDrift => "on_drift",
            Event::OnGuardRefusal => "on_guard_refusal",
        }
    }

    pub fn parse(name: &str) -> Option<Event> {
        Event::ALL.into_iter().find(|e| e.as_str() == name)
    }
}

impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

/// What a hook is told, as JSON on stdin.
///
/// One envelope for every event — the `event` field says which, and `data` carries what that
/// event knows. A hook that handles two events therefore parses one shape, and a hook that
/// handles one can still tell which it got.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Payload {
    pub schema: u32,
    pub event: &'static str,
    pub data: serde_json::Value,
}

impl Payload {
    pub fn new(event: Event, data: serde_json::Value) -> Payload {
        Payload {
            schema: PAYLOAD_SCHEMA,
            event: event.as_str(),
            data,
        }
    }

    /// The bytes written to the hook's stdin. Newline-terminated so a hook that reads a line
    /// at a time — `read -r line`, `for line in sys.stdin` — sees a complete record.
    pub fn to_stdin(&self) -> String {
        // The payload is built from owned data with string keys, so this cannot fail; if it
        // ever did, an empty object is a shape the hook can still parse.
        let body = serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string());
        format!("{}\n", body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_event_round_trips_through_its_name() {
        for event in Event::ALL {
            assert_eq!(Event::parse(event.as_str()), Some(event));
        }
    }

    /// A misspelled event must not parse into a real one. A hook that silently never fires is
    /// the worst outcome for something whose job is to tell you.
    #[test]
    fn an_unknown_event_does_not_parse() {
        assert_eq!(Event::parse("after-sync"), None);
        assert_eq!(Event::parse("AFTER_SYNC"), None);
        assert_eq!(Event::parse(""), None);
        assert_eq!(Event::parse("on_drifts"), None);
    }

    #[test]
    fn the_payload_is_versioned_and_names_its_event() {
        let p = Payload::new(Event::OnDrift, json!({"removed": ["jq"]}));
        let out = p.to_stdin();
        assert!(out.contains("\"schema\":1"), "{}", out);
        assert!(out.contains("\"event\":\"on_drift\""), "{}", out);
        assert!(out.contains("\"removed\":[\"jq\"]"), "{}", out);
    }

    /// A hook that reads a line at a time must see a complete record.
    #[test]
    fn stdin_is_newline_terminated() {
        let out = Payload::new(Event::AfterSync, json!({})).to_stdin();
        assert!(out.ends_with('\n'), "{:?}", out);
        assert_eq!(out.lines().count(), 1, "{:?}", out);
    }

    /// One envelope for every event, so a hook handling two parses one shape.
    #[test]
    fn every_event_produces_the_same_envelope() {
        for event in Event::ALL {
            let v: serde_json::Value =
                serde_json::from_str(&Payload::new(event, json!({})).to_stdin()).unwrap();
            assert_eq!(v["schema"], 1);
            assert_eq!(v["event"], event.as_str());
            assert!(v.get("data").is_some());
        }
    }
}
