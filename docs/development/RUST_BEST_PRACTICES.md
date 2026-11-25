# Rust Best Practices

Basnet's standing Rust rule (adopted 2026-11-17), committed with
the code it governs. The working copy the assistant follows lives
in the custom skill `rust_best_practices`; this document is the
project-side copy for this repository. Distilled from the full
per-file read audit of 2026-11-17 (all 309 Rust files) and the
optimisation pass that preceded it.

## Priorities

- Basnet's order: security first, then speed, then architecture,
  then readability. When two conflict, the earlier one wins.
- Correctness before optimisation. An optimisation's gain is
  claimed only with a measurement, or labelled "by construction"
  when it follows from the code shape (a clone removed, a query
  round-trip eliminated). Never invent benchmark numbers.
- Keep the dependency list small. No new crate for a
  micro-optimisation without a measured hotspot (the FxHashMap /
  SmallVec lesson: rejected, no hotspot), and no new abstraction
  without a session decision.

## Errors and failure behaviour

- Never swallow an error silently. Every `.ok()`, `let _ =` and
  `unwrap_or_default()` on a fallible operation either logs or
  carries a comment stating why silence is safe. A fallback is a
  decision: log it at the point of decision, once, with the
  record's ID.
- Fail closed on security, money and cap paths. A failed count
  reads as the cap reached (never as "no cap"); a failed stamp
  read reads as a value no real stamp equals; a guard whose
  write did not persist refuses the operation it was guarding.
- Errors are logged once, at the layer that creates the error
  value, with context. A `From` conversion used with `?` must
  honour the same logging contract — a conversion that bypasses
  the log is a defect. User-facing messages never carry internal
  detail.
- Panics are for genuine invariants only, and the invariant is
  documented at the site. Data read paths use checked variants:
  one corrupt or undecryptable row must never panic a request,
  and never kill a background sweep — the sweep logs, reverts or
  parks the row's claim, and continues with the next row.
- Startup validates what the process will lazily need (secrets,
  config ranges) and refuses to start with a named message,
  instead of panicking later on first use. Config validation
  covers every numeric knob, not most of them.

## Concurrency and races

- A guard must ride the mutation itself: a compare-and-swap
  condition in the UPDATE's WHERE clause, a recount inside the
  transaction, a per-owner advisory lock for exactly-once
  applies. A check performed before or outside the transaction
  that the write does not re-verify is a defect, however careful
  the check looks.
- No lock or mutex is held across an `.await`. Lock access
  recovers from poisoning instead of panicking. When an advisory
  unlock fails, close that connection rather than returning a
  still-locked connection to the pool.
- No database transaction stays open across a network call:
  commit first, call second, compensate on failure.

## Numbers and money

- Money is integer minor units end to end (yen as i64 in
  Basnet's apps); floats never carry money. Money columns are
  BIGINT so sqlx decodes them into i64 on read-back.
- Checked arithmetic wherever a number came from user input or
  parsing — a typed amount whose unit multiplication overflows
  i64 is a reachable panic in debug and a wrapped, wrong value
  in release. Use `checked_*` at parse boundaries and i128
  intermediates for multiply-then-divide on large values.
  Overflow at an input boundary is a bug class to test, not an
  edge case to ignore.

## Data access

- Explicit column lists, never `SELECT *` — a shared `*_COLUMNS`
  constant per table. Bound parameters only, everywhere.
- Fetch only the columns and rows the caller needs: list pages
  do not load long text columns, and a search that must filter
  in memory says so in a comment with its scale bound.
- Bulk loads (`ANY($1)`) over per-row query loops on hot paths;
  per-row writes inside one transaction are fine. Lists
  paginate; sweep work lists carry a batch cap.
- Migrations are the schema's only source of truth. After
  adding a migration, force a recompile before running, or an
  embedded-migrations build silently skips it.

## Ownership, allocation and async

- Borrow first; clone only with a reason. Return `Cow` when a
  value is usually borrowed or static. Static data is a
  `&'static` slice or const — never a per-request `Vec<String>`
  rebuilt from it. Consume owned values instead of cloning out
  of them.
- No blocking IO or CPU-heavy work on Tokio workers: image and
  other CPU work goes through `spawn_blocking`, file reads
  through the async API, and no synchronous filesystem calls
  hide inside handlers.
- Independent awaits on a hot path run concurrently (`join!` /
  `try_join!`); sequential awaits are for data-dependent steps
  and transaction bodies, and say so.

## Structure and duplication

- One definition for every shared mechanism, constant and
  message. A copy-pasted helper (an error lookup, a formatting
  body, an error type) is a defect: when work touches it, unify
  it in the same change (refactor-on-the-way, scoped to what the
  change touches). Semantics that differ per copy are preserved
  per copy — unification never silently flattens a real
  difference.
- Layer flow is one-directional (controllers → services →
  repositories → models in the EC shape). A controller calling a
  repository directly is a finding, not a shortcut.
- Middleware and layers are ordered deliberately and the order
  is verified against the framework's semantics — in axum, a
  layer applied before a `.merge` does not reach the merged-in
  routes. A claim like "set on every response" is checked
  against every route group, including health and webhooks.
- Long functions that mix validation, transaction work and IO
  get split at those seams when touched, keeping transaction
  boundaries and lock order exactly as they were.

## Docs and comments

- Every file opens with a `//!` module doc; every public item
  carries a `///` doc. A doc comment belongs to the item
  directly below it — a doc displaced onto the wrong field by a
  later insertion is a defect, as is an orphaned doc for a field
  that no longer exists.
- A doc claim is a contract: if the doc promises logging,
  headers, a guard or a fallback, the code must deliver it.
  When the code and the doc disagree, one of them is the bug —
  decide which, fix it, and say which in the report.
- Comments state the rule or reason that still holds today, in
  the fewest lines that keep the meaning. History belongs in
  git and the audit docs.

## Verification

- The project's gate (fmt, clippy with warnings denied, tests,
  doc, audit) is green before work is presented as done.
- A changed SQL shape is executed against the dev database
  before it is trusted, not just compiled.
- Coverage claims mean what they say: "scanned" is a grep,
  "read" is a file read end to end. Report which one was done,
  with counts.
- Rejected techniques stay rejected without new evidence —
  record the rejection and its reason where the next session
  will see it, instead of re-proposing on a fresh brief.
