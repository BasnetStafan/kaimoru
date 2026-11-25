# Shared harness for the scripts/tests/test_*.sh suites (V1
# perfection pass ST-6): the pass/fail counters, the RESULT
# line and the CSRF-token scrape that every suite used to
# define for itself — twelve near-identical copies, differing
# just enough to drift.
#
# Usage: a suite sources this file near the top, after its
# BASE definition:
#   . "$(dirname "$0")/lib.sh"
# Suites whose checks only echo (the stage suites print no
# RESULT line) set SUITE_NO_COUNT=1 before sourcing; their
# output stays exactly as it was — bare OK/NG lines, no
# counters. test_design_fixes.sh sources this file for
# csrf() only: its ok() takes an exit code instead of a
# label-first pair, so it keeps its own definition after
# the source line (the later definition wins).
PASS=0; FAIL=0
if [ "${SUITE_NO_COUNT:-0}" = "1" ]; then
ok() { echo "  OK  $1"; }
ng() { echo "  NG  $1"; }
else
ok() { PASS=$((PASS+1)); echo "  OK  $1"; }
ng() { FAIL=$((FAIL+1)); echo "  NG  $1"; }
fi
# The session's CSRF token out of a saved page: the hidden
# field the middleware injects into every non-GET form.
csrf() { grep -o 'name="csrf_token" value="[^"]*"' "$1" | head -1 | sed 's/.*value="//; s/"$//'; }
# The one-time form ticket a guarded edit form carries (the
# staleness guard): scraped from the rendered edit page,
# exactly like the CSRF token.
ticket() { grep -o 'name="form_ticket" value="[^"]*"' "$1" | head -1 | sed 's/.*value="//; s/"$//'; }
# The closing line of the counting suites.
result() { echo "RESULT: $PASS passed, $FAIL failed"; }

# The users.email_bidx blind index for an email, derived
# exactly as app/src/pii.rs derives it (HMAC-SHA-256 keyed by
# a key HMAC-derived from the PII master key under a domain
# string, over "email:" + the trimmed, lowercased address).
# Suites look members up by it now that the email column
# holds ciphertext. Dev box only: the key comes from the
# project .env, like every other local secret.
bidx() {
  PII_ENV_FILE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/.env" \
  python3 - "$1" <<'PYEOF'
import base64, hashlib, hmac, os, pathlib, sys
key = None
for line in pathlib.Path(os.environ["PII_ENV_FILE"]).read_text().splitlines():
    if line.startswith("PII_ENCRYPTION_KEY="):
        key = base64.b64decode(line.split("=", 1)[1].strip())
if key is None:
    sys.exit("PII_ENCRYPTION_KEY not found in .env")
blind = hmac.new(key, b"pii-blind-index-v1", hashlib.sha256).digest()
email = sys.argv[1].strip().lower()
print(hmac.new(blind, ("email:" + email).encode(), hashlib.sha256).hexdigest())
PYEOF
}

# Decrypts one field stored by the app (the "v1:" format of
# app/src/pii.rs) back to plaintext, so suites can keep
# asserting on the values the app was given. Anything not in
# v1: form (direct-SQL fixture rows, scrub markers) passes
# through unchanged, mirroring the app's own read path.
# Dev box only: the key comes from the project .env.
pii_dec() {
  PII_ENV_FILE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/.env" \
  PII_VAL="$1" \
  python3 - <<'PYEOF'
import base64, os, pathlib, sys
v = os.environ["PII_VAL"]
if not v.startswith("v1:"):
    print(v)
    sys.exit(0)
key = None
for line in pathlib.Path(os.environ["PII_ENV_FILE"]).read_text().splitlines():
    if line.startswith("PII_ENCRYPTION_KEY="):
        key = base64.b64decode(line.split("=", 1)[1].strip())
if key is None:
    sys.exit("PII_ENCRYPTION_KEY not found in .env")
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
raw = base64.b64decode(v[3:])
print(AESGCM(key).decrypt(raw[:12], raw[12:], None).decode())
PYEOF
}

# The app refuses to boot without the PII master key
# (configuration::Config::pii_encryption_key); suites that
# launch the binary directly inherit it from here. Read from
# the project .env at source time; never printed.
if [ -z "${PII_ENCRYPTION_KEY:-}" ]; then
  _pii_env_file="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/.env"
  if [ -f "$_pii_env_file" ]; then
    PII_ENCRYPTION_KEY="$(sed -n 's/^PII_ENCRYPTION_KEY=//p' "$_pii_env_file" | head -1)"
    export PII_ENCRYPTION_KEY
  fi
  unset _pii_env_file
fi
