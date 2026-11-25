//! One-time form tickets: the transport for this app's
//! Optimistic Offline Locks on edit forms.
//!
//! The problem is the lost update over think-time: a form is
//! rendered from a record, the human edits for minutes, and
//! the submit writes absolute values — a second tab (or a
//! second staff member) that saved in between is silently
//! overwritten. The guard needs the record's `updated_at`
//! from render time at submit time. Carrying the stamp in a
//! hidden field exposes record state to the client and lets
//! it be edited; instead each render mints a one-time random
//! ticket, and the session keeps the ticket note — scope and
//! stamp — server-side. The form carries only the ticket: a
//! forged or replayed ticket matches nothing and is refused,
//! and a ticket minted for one record never validates a save
//! for another, because the scope is part of the note.
//!
//! The ticket is only the transport. The correctness check
//! is the one the admin product save has always run: the
//! service re-reads the record's `updated_at` under
//! `FOR UPDATE` inside the save transaction and compares it
//! with the ticket's stamp, refusing with
//! [`AppError::Conflict`] on a mismatch.
//!
//! Tickets are capped at ten per session: renders that are
//! never submitted age out oldest-first, so opening edit
//! pages all afternoon cannot grow the session without
//! bound.

use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tower_sessions::Session;

use crate::error::AppError;

/// The session key the ticket notes live under.
const SESSION_FORM_TICKETS: &str = "form_tickets";

/// How many outstanding tickets one session keeps: two tabs,
/// a validation re-render or two, and slack. Beyond this the
/// oldest render's ticket stops validating and its submit is
/// refused as stale, which a page reload cures.
const MAX_TICKETS: usize = 10;

/// One ticket note: the record it was minted for (the scope,
/// e.g. `profile` or `address:42`) and the record's
/// `updated_at` at render time, as epoch microseconds — the
/// representation the product guard's `expected` uses.
#[derive(Clone, Serialize, Deserialize)]
struct FormTicket {
    token: String,
    scope: String,
    stamp: i64,
}

/// Mints a ticket for `scope` at `stamp`, records it in the
/// session, and returns the token for the form's hidden
/// field.
pub async fn issue(session: &Session, scope: &str, stamp: i64) -> String {
    let token = new_token();
    let mut tickets = load(session).await;
    tickets.push(FormTicket {
        token: token.clone(),
        scope: scope.to_string(),
        stamp,
    });
    if tickets.len() > MAX_TICKETS {
        let excess = tickets.len() - MAX_TICKETS;
        tickets.drain(..excess);
    }
    if let Err(e) = session.insert(SESSION_FORM_TICKETS, tickets).await {
        // The minted token will simply never validate (consume
        // cannot find it in the stored session); log so a
        // session-store failure is visible instead of surfacing
        // as a mystery stale-form refusal.
        tracing::error!("form ticket issue failed to record: {e}");
    }
    token
}

/// Spends the ticket named by `token` for `scope`: removes
/// its note and returns the recorded stamp, or `None` when
/// the session holds no such ticket (an expired session, a
/// replayed ticket, or a ticket minted for another record).
/// A matched ticket is spent by its first submit, successful
/// or not — the re-rendered form carries a fresh one.
pub async fn consume(session: &Session, scope: &str, token: &str) -> Option<i64> {
    if token.is_empty() {
        return None;
    }
    let mut tickets = load(session).await;
    let position = tickets
        .iter()
        .position(|ticket| ticket.token == token && ticket.scope == scope)?;
    let ticket = tickets.remove(position);
    if let Err(e) = session.insert(SESSION_FORM_TICKETS, tickets).await {
        // The spend did not persist, so the stored session
        // still holds the ticket: honouring the stamp here
        // would leave it replayable. Fail closed — the save is
        // refused as stale and the re-rendered form carries a
        // fresh ticket.
        tracing::error!("form ticket spend failed to record: {e}");
        return None;
    }
    Some(ticket.stamp)
}

/// The session's outstanding ticket notes.
async fn load(session: &Session) -> Vec<FormTicket> {
    session
        .get(SESSION_FORM_TICKETS)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// One ticket token: 32 random bytes as hex — the
/// unsubscribe-token shape, unguessable and meaningless
/// outside the session that minted it.
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The refusal every form-ticket guard raises when the
/// record moved after the form was rendered: the shared
/// wording lives in [`crate::messages`]; this constructor
/// keeps the call sites one line.
pub fn stale_conflict() -> AppError {
    AppError::Conflict(crate::messages::STALE_FORM_MESSAGE.to_string())
}

/// The staleness check the record guards run inside the
/// save transaction: `current` is the record's `updated_at`
/// re-read under `FOR UPDATE` (the product guard's shape),
/// `expected` the stamp the spent ticket recorded at render
/// time. A missing row is stale too — the record the form
/// was rendered from is gone. Two guards keep their own
/// comparison because their missing-row meaning differs:
/// the notification override (no row is the stamp 0) and
/// the address save (no row falls through to not-found).
pub fn ensure_fresh(current: Option<DateTime<Utc>>, expected: i64) -> Result<(), AppError> {
    if current.map(|t| t.timestamp_micros()) == Some(expected) {
        Ok(())
    } else {
        Err(stale_conflict())
    }
}

/// A deterministic builder for set-shaped stamps: a
/// surface whose form replaces a whole set (the menus
/// editor, the home content editor) has no single row
/// stamp, so its ticket stamp is a fingerprint of the set
/// the form was rendered from. FNV-1a over
/// length-prefixed fields — stable across runs and
/// processes, unlike the standard library's keyed hasher.
pub struct SetStamp(u64);

impl SetStamp {
    /// An empty stamp builder.
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
    }

    /// Mixes one text field, length first so field
    /// boundaries can never blur.
    pub fn text(&mut self, value: &str) {
        self.int(value.len() as i64);
        for byte in value.as_bytes() {
            self.byte(*byte);
        }
    }

    /// Mixes one integer field.
    pub fn int(&mut self, value: i64) {
        for byte in value.to_le_bytes() {
            self.byte(byte);
        }
    }

    /// Mixes one switch field.
    pub fn flag(&mut self, value: bool) {
        self.byte(u8::from(value));
    }

    /// The stamp value, carried by the form ticket.
    pub fn finish(&self) -> i64 {
        self.0 as i64
    }
}

impl Default for SetStamp {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::SetStamp;

    /// Builds a stamp from text fields only.
    fn stamp_of(parts: &[&str]) -> i64 {
        let mut stamp = SetStamp::new();
        for part in parts {
            stamp.text(part);
        }
        stamp.finish()
    }

    #[test]
    fn set_stamp_is_deterministic() {
        assert_eq!(stamp_of(&["menu", "ホーム"]), stamp_of(&["menu", "ホーム"]));
        assert_ne!(stamp_of(&["menu"]), stamp_of(&["other"]));
    }

    #[test]
    fn set_stamp_keeps_field_boundaries() {
        // The length prefix keeps the split point visible:
        // "ab" + "c" must not fingerprint as "a" + "bc",
        // or a stale form could pass the guard after a
        // neighbouring field absorbed a character.
        assert_ne!(stamp_of(&["ab", "c"]), stamp_of(&["a", "bc"]));
        assert_ne!(stamp_of(&["ab"]), stamp_of(&["a", "b"]));
    }

    #[test]
    fn set_stamp_reacts_to_int_and_flag_fields() {
        let build = |n: i64, on: bool| {
            let mut stamp = SetStamp::new();
            stamp.text("x");
            stamp.int(n);
            stamp.flag(on);
            stamp.finish()
        };
        assert_ne!(build(1, true), build(1, false));
        assert_ne!(build(1, true), build(2, true));
        assert_eq!(build(1, true), build(1, true));
    }
}
