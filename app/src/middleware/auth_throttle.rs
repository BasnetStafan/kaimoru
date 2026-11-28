//! In-memory login throttling, plus start throttling for the
//! mail-sending flows (sign-up, password reset).
//!
//! Disposable by design: a restart clears these counters. Limits
//! that must survive a restart (sign-up code attempts and expiry)
//! are enforced in the database instead. Nothing here stores an
//! email beyond the in-memory map key.
//!
//! Deployment note: counters are per process, so on ECS the
//! effective limits multiply by the task count. Accepted for v1 —
//! the database-enforced code limits still hold.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::util::rate_limit_knobs::SWEEP_THRESHOLD;

/// One (email, IP) pair locks after this many failures...
const PAIR_LIMIT: usize = 5;
/// ...inside this window...
const PAIR_WINDOW: Duration = Duration::from_secs(15 * 60);
/// ...for this long. Attempts during a lock do not extend it.
const PAIR_LOCK: Duration = Duration::from_secs(15 * 60);
/// One IP locks after this many failures across all accounts...
const IP_LIMIT: usize = 20;
/// ...inside this window...
const IP_WINDOW: Duration = Duration::from_secs(60 * 60);
/// ...for this long.
const IP_LOCK: Duration = Duration::from_secs(60 * 60);

/// Failure timestamps plus an optional active lock, for one key.
#[derive(Default)]
struct Attempts {
    /// Recent failure times inside the counting window.
    failures: Vec<Instant>,
    /// When the current lock ends, if locked.
    locked_until: Option<Instant>,
}

impl Attempts {
    /// Whether this key is locked right now.
    fn is_locked(&self, now: Instant) -> bool {
        self.locked_until.is_some_and(|until| until > now)
    }

    /// Counts one failure and locks when the limit is reached.
    /// A failure during an active lock changes nothing, so the lock
    /// always ends when it was first scheduled to.
    fn record_failure(&mut self, now: Instant, window: Duration, limit: usize, lock: Duration) {
        if self.is_locked(now) {
            return;
        }
        self.failures.retain(|t| now.duration_since(*t) < window);
        self.failures.push(now);
        if self.failures.len() >= limit {
            self.locked_until = Some(now + lock);
            self.failures.clear();
        }
    }
}

/// All throttle state: one entry per (email, IP) pair and per IP.
#[derive(Default)]
struct Throttle {
    pairs: HashMap<(String, String), Attempts>,
    ips: HashMap<String, Attempts>,
}

/// Whether an entry carries nothing worth keeping: not locked,
/// and every failure older than the counting window.
fn attempts_dead(attempts: &Attempts, now: Instant, window: Duration) -> bool {
    !attempts.is_locked(now)
        && attempts
            .failures
            .iter()
            .all(|t| now.duration_since(*t) >= window)
}

/// Drops the dead entries of one attempts map.
fn sweep_attempts<K: std::hash::Hash + Eq>(
    map: &mut HashMap<K, Attempts>,
    now: Instant,
    window: Duration,
) {
    map.retain(|_, attempts| !attempts_dead(attempts, now, window));
}

/// Drops the dead entries of one timestamp-list map.
fn sweep_timestamps<K: std::hash::Hash + Eq>(
    map: &mut HashMap<K, Vec<Instant>>,
    now: Instant,
    window: Duration,
) {
    map.retain(|_, times| {
        times.retain(|t| now.duration_since(*t) < window);
        !times.is_empty()
    });
}

/// The member login throttle state.
static THROTTLE: LazyLock<Mutex<Throttle>> = LazyLock::new(|| Mutex::new(Throttle::default()));

/// The admin login throttle state — separate from the member
/// side, so locking one side never locks the other.
static ADMIN_THROTTLE: LazyLock<Mutex<Throttle>> =
    LazyLock::new(|| Mutex::new(Throttle::default()));

/// Locks one throttle state, recovering from a poisoned mutex:
/// throttling must never take login down with it.
fn lock_throttle(
    throttle: &'static LazyLock<Mutex<Throttle>>,
) -> std::sync::MutexGuard<'static, Throttle> {
    throttle
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Normalizes an email for use as a counter key.
fn key_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Whether a login attempt for this (email, IP) must be refused
/// before the credentials are even checked.
fn is_blocked_in(state: &Throttle, email: &str, ip: &str) -> bool {
    let now = Instant::now();
    let pair = (key_email(email), ip.to_string());
    state.pairs.get(&pair).is_some_and(|a| a.is_locked(now))
        || state.ips.get(ip).is_some_and(|a| a.is_locked(now))
}

/// Records one failed login against the pair counter and the IP
/// counter, locking either when its limit is reached.
fn record_failure_in(state: &mut Throttle, email: &str, ip: &str) {
    let now = Instant::now();
    if state.pairs.len() >= SWEEP_THRESHOLD {
        sweep_attempts(&mut state.pairs, now, PAIR_WINDOW);
    }
    if state.ips.len() >= SWEEP_THRESHOLD {
        sweep_attempts(&mut state.ips, now, IP_WINDOW);
    }
    state
        .pairs
        .entry((key_email(email), ip.to_string()))
        .or_default()
        .record_failure(now, PAIR_WINDOW, PAIR_LIMIT, PAIR_LOCK);
    state
        .ips
        .entry(ip.to_string())
        .or_default()
        .record_failure(now, IP_WINDOW, IP_LIMIT, IP_LOCK);
}

/// Clears the (email, IP) pair counter after a successful login.
/// The IP counter is kept on purpose: one valid account must not
/// reset the evidence of a wider attack from the same address.
fn record_success_in(state: &mut Throttle, email: &str, ip: &str) {
    state.pairs.remove(&(key_email(email), ip.to_string()));
}

/// Whether a member login attempt must be refused up front.
pub fn is_blocked(email: &str, ip: &str) -> bool {
    is_blocked_in(&lock_throttle(&THROTTLE), email, ip)
}

/// Records one failed member login.
pub fn record_failure(email: &str, ip: &str) {
    record_failure_in(&mut lock_throttle(&THROTTLE), email, ip);
}

/// Clears the member pair counter after a successful login.
pub fn record_success(email: &str, ip: &str) {
    record_success_in(&mut lock_throttle(&THROTTLE), email, ip);
}

/// Whether an admin login attempt must be refused up front.
pub fn admin_is_blocked(email: &str, ip: &str) -> bool {
    is_blocked_in(&lock_throttle(&ADMIN_THROTTLE), email, ip)
}

/// Records one failed admin login.
pub fn admin_record_failure(email: &str, ip: &str) {
    record_failure_in(&mut lock_throttle(&ADMIN_THROTTLE), email, ip);
}

/// Clears the admin pair counter after a successful login.
pub fn admin_record_success(email: &str, ip: &str) {
    record_success_in(&mut lock_throttle(&ADMIN_THROTTLE), email, ip);
}

/// The guest order-lookup throttle state — its own instance,
/// so lookup abuse never locks anyone's login, and a locked
/// login never blocks a lookup.
static LOOKUP_THROTTLE: LazyLock<Mutex<Throttle>> =
    LazyLock::new(|| Mutex::new(Throttle::default()));

/// Whether a guest order lookup must be refused up front:
/// the submitted pair is the whole credential, so failed
/// matches throttle exactly like failed logins.
pub fn lookup_is_blocked(email: &str, ip: &str) -> bool {
    is_blocked_in(&lock_throttle(&LOOKUP_THROTTLE), email, ip)
}

/// Records one failed guest order lookup.
pub fn lookup_record_failure(email: &str, ip: &str) {
    record_failure_in(&mut lock_throttle(&LOOKUP_THROTTLE), email, ip);
}

/// Clears the lookup pair counter after a successful match.
pub fn lookup_record_success(email: &str, ip: &str) {
    record_success_in(&mut lock_throttle(&LOOKUP_THROTTLE), email, ip);
}

/// Flow starts allowed per email inside the window.
const START_EMAIL_LIMIT: usize = 5;
/// Flow starts allowed per IP inside the window.
const START_IP_LIMIT: usize = 10;
/// The flow start window.
const START_WINDOW: Duration = Duration::from_secs(60 * 60);

/// Start times of recent mail-sending flow starts, per key.
/// One instance per flow, so one flow's traffic can never eat
/// another flow's budget.
#[derive(Default)]
struct StartLog {
    by_email: HashMap<String, Vec<Instant>>,
    by_ip: HashMap<String, Vec<Instant>>,
}

/// Sign-up start state, beside the login throttle.
static SIGNUP_STARTS: LazyLock<Mutex<StartLog>> = LazyLock::new(|| Mutex::new(StartLog::default()));

/// Password reset start state: same limits, own counters.
static RESET_STARTS: LazyLock<Mutex<StartLog>> = LazyLock::new(|| Mutex::new(StartLog::default()));

/// Locks one start log, recovering from a poisoned mutex.
fn lock_start_log(
    log: &'static LazyLock<Mutex<StartLog>>,
) -> std::sync::MutexGuard<'static, StartLog> {
    log.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether one more start is allowed in one flow's log for this
/// email from this IP, recording the start when allowed.
fn start_allowed_in(state: &mut StartLog, email: &str, ip: &str) -> bool {
    let now = Instant::now();
    if state.by_email.len() >= SWEEP_THRESHOLD {
        sweep_timestamps(&mut state.by_email, now, START_WINDOW);
    }
    if state.by_ip.len() >= SWEEP_THRESHOLD {
        sweep_timestamps(&mut state.by_ip, now, START_WINDOW);
    }
    let email_key = key_email(email);
    {
        let entry = state.by_email.entry(email_key.clone()).or_default();
        entry.retain(|t| now.duration_since(*t) < START_WINDOW);
        if entry.len() >= START_EMAIL_LIMIT {
            return false;
        }
    }
    {
        let entry = state.by_ip.entry(ip.to_string()).or_default();
        entry.retain(|t| now.duration_since(*t) < START_WINDOW);
        if entry.len() >= START_IP_LIMIT {
            return false;
        }
    }
    state.by_email.entry(email_key).or_default().push(now);
    state.by_ip.entry(ip.to_string()).or_default().push(now);
    true
}

/// Whether one more sign-up may start for this email from this IP,
/// recording the start when allowed.
///
/// Starting a sign-up sends a real email; without this limit the
/// form could mail-bomb arbitrary addresses.
pub fn signup_start_allowed(email: &str, ip: &str) -> bool {
    start_allowed_in(&mut lock_start_log(&SIGNUP_STARTS), email, ip)
}

/// Whether one more password reset may start for this email from
/// this IP, recording the start when allowed.
///
/// A reset sends a real email too, so it carries the same limits
/// in its own counters. Every submitted address is counted,
/// member or not: the throttle must never reveal which addresses
/// have accounts.
pub fn reset_start_allowed(email: &str, ip: &str) -> bool {
    start_allowed_in(&mut lock_start_log(&RESET_STARTS), email, ip)
}
