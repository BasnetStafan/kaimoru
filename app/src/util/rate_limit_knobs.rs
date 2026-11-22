//! Shared knobs for the app's in-memory rate limiters.
//!
//! Every limiter keeps its counters in a process-local map —
//! login attempts, inquiry submissions, search suggestions,
//! review posts. The maps are disposable by design (a restart
//! resets them), but each one needs the same bound on growth,
//! which lives here exactly once.

/// Above this many tracked keys a limiter map sweeps its dead
/// entries before a new one is inserted: without the sweep, a
/// client rotating emails, IPs or accounts could grow a map
/// without bound.
pub(crate) const SWEEP_THRESHOLD: usize = 10_000;
