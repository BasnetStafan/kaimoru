# AGENTS.md

Guidance for AI coding agents (and humans) working in this repository.

## What this is

Kaimoru is a full-stack, server-rendered e-commerce store built
complete for Japan, written in Rust (Axum) with PostgreSQL.
See `README.md` for the product overview and
`docs/development/ARCHITECTURE.md` for the system design.

## Layout

- `server/` — binary entry point: logging, bind, graceful shutdown.
- `app/` — the web layer. Dependencies run one way:
  `controllers/` → `services/` → `repositories/` → `models/`.
  Supporting code lives in `views/`, `middleware/`, `util/`.
- `configuration/` — typed config from `config.toml`. Secrets
  never live here; they go in `.env` (see `.env.example`).
- `resources/templates/` — Askama templates.
  `resources/static/` — self-hosted CSS/JS/images, served
  under `/static`.
- `migrations/` — sqlx migrations, applied at startup.
- `scripts/` — `check.sh` (quality gate), `tests/` (E2E suites),
  `stubs/`, `ops/`.

## Quality gate

Run this before considering any change done:

```bash
./scripts/check.sh
```

It runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, `cargo doc` (warnings denied) and `cargo audit`.
Everything must pass.

If you add, rename or remove a unit test, refresh the inventory
and verify it stays in sync with the code:

```bash
bash scripts/ops/unit_test_spec.sh generate
bash scripts/ops/unit_test_spec.sh check
```

## Conventions that are not negotiable

- **Money is integer yen, `i64` end to end** (BIGINT in the
  database). Floats never touch a price. A change to a money
  rule (tax, points, fees) updates the matching spec in
  `docs/specs/` in the same change.
- **Migrations are never edited after application.** Add a new
  migration instead. After adding one, `touch app/src/lib.rs`
  so the embedded migration set is recompiled.
- **UI text is Japanese.** Code, comments and developer docs
  are English; user-facing documents are Japanese.
- Every Rust file starts with a `//!` module doc; public items
  carry `///` docs (the gate's `cargo doc` denies warnings).
- Repeated literals become named constants, or are centralised
  in `app/src/messages.rs`.
- Prefer the existing dependency set. Do not add a crate or a
  new abstraction without a stated reason.
- Layering is one-directional: business rules in services, SQL
  in repositories, no skipping a layer.
- Logs never contain email addresses, verification codes,
  addresses or request bodies — request IDs and record IDs only.
- Never commit `.env` or any real secret. `.env.example`
  carries placeholders only.

## Running locally

```bash
cp .env.example .env   # fill in the four required values
./run.sh               # http://localhost:3000 (Mailpit at :8025)
```

Login-heavy E2E suites share the app's in-memory login
throttle; restart the app between such runs. See
`docs/development/TESTING.md` for the suite inventory and
prerequisites.
