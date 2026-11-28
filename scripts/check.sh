#!/usr/bin/env bash
# The full local gate (code review R2-16): format, lints, unit
# tests, documentation and the dependency audit — the same
# checks a CI run would make, in one command. Run before
# asking for a review or committing.
#
# Requires cargo-audit once: cargo install cargo-audit --locked
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== cargo fmt --check =="
cargo fmt --check

echo "== cargo clippy =="
cargo clippy --all-targets -- -D warnings

echo "== cargo test =="
cargo test

echo "== cargo doc =="
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

echo "== cargo audit =="
cargo audit

echo "ALL CHECKS PASSED"
