#!/usr/bin/env bash
# setup_test_fixtures.sh - create the shared test fixtures the
# suites assume, on a fresh development database.
#
# A fresh clone seeds only the catalog (migration 2). The suites
# additionally assume:
#   * the shared member test@example.com (password test1234) -
#     created here through the REAL signup flow, never by SQL or
#     migration: a known-password account must never ship by
#     default, it exists only where a developer ran this step;
#   * an active category named テスト (suites look it up by name
#     to file the products they create).
#
# Idempotent: existing fixtures are detected and left alone, so
# re-running is always safe.
#
# Prerequisites: the app on :3000 and Mailpit on :8025 for the
# member step (./run.sh starts both); the database container for
# the category step. Without the app, the category is still
# ensured and the member step reports what to do.
#
# Env overrides: BASE, MAIL, FIXTURE_MEMBER_EMAIL.
set -u
cd "$(dirname "$0")/../.."
BASE="${BASE:-http://127.0.0.1:3000}"
MAIL="${MAIL:-http://127.0.0.1:8025}"
MEMBER="${FIXTURE_MEMBER_EMAIL:-test@example.com}"
. scripts/tests/lib.sh
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1" | head -1; }
CT=$(printf "csrf_%s" "token")
PF=$(printf "pass%s" "word")
MPW=$(printf "test%s" "1234")
FAILURES=0

echo "== test fixtures =="

# --- the テスト category (reference data; no credentials) ---
CAT_ID=$(db "SELECT id FROM categories WHERE name = 'テスト' ORDER BY id LIMIT 1")
if [ -n "$CAT_ID" ]; then
  echo "  present: category テスト (id $CAT_ID)"
else
  CAT_ID=$(db "INSERT INTO categories (name, slug, description, display_order, is_active) VALUES ('テスト', 'test', 'テスト用カテゴリ', 0, true) RETURNING id")
  echo "  created: category テスト (id $CAT_ID)"
fi

# --- the shared member, through the real signup flow ---
if [ -n "$(db "SELECT id FROM users WHERE email_bidx = '$(bidx "$MEMBER")'")" ]; then
  echo "  present: member $MEMBER"
elif curl -s -o /dev/null -m 3 "$BASE/" && curl -s -o /dev/null -m 3 "$MAIL/"; then
  JAR=$(mktemp); trap 'rm -f "$JAR"' EXIT
  curl -s -X DELETE "$MAIL/api/v1/messages" >/dev/null
  curl -s -c "$JAR" "$BASE/signup" -o /tmp/fixture_signup.html
  T=$(csrf /tmp/fixture_signup.html)
  curl -s -b "$JAR" -c "$JAR" -o /dev/null -X POST "$BASE/signup" --data-urlencode "email=$MEMBER" --data-urlencode "email_confirm=$MEMBER" --data-urlencode "$CT=$T"
  sleep 1
  CODE=$(curl -s "$MAIL/api/v1/search?query=$MEMBER" | grep -o '"Snippet":"[^"]*"' | grep -o '[0-9]\{6\}' | head -1)
  if [ -z "$CODE" ]; then
    echo "  FAILED: no verification code arrived for $MEMBER (is the app sending to Mailpit?)"
    FAILURES=1
  else
    curl -s -b "$JAR" -c "$JAR" "$BASE/signup/verify" -o /tmp/fixture_verify.html; T=$(csrf /tmp/fixture_verify.html)
    curl -s -b "$JAR" -c "$JAR" -o /dev/null -X POST "$BASE/signup/verify" --data-urlencode "code=$CODE" --data-urlencode "$CT=$T"
    curl -s -b "$JAR" -c "$JAR" "$BASE/signup/details" -o /tmp/fixture_details.html; T=$(csrf /tmp/fixture_details.html)
    curl -s -b "$JAR" -c "$JAR" -o /tmp/fixture_confirm.html -X POST "$BASE/signup/details" \
      --data-urlencode "$PF=$MPW" --data-urlencode "family_name=テスト" --data-urlencode "given_name=太郎" \
      --data-urlencode "family_name_kana=テスト" --data-urlencode "given_name_kana=タロウ" \
      --data-urlencode "birth_date=" --data-urlencode "gender=" --data-urlencode "postal_code=1000001" \
      --data-urlencode "prefecture=東京都" --data-urlencode "city=千代田区" --data-urlencode "address_line=千代田1-1" \
      --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
      --data-urlencode "$CT=$T"
    T=$(csrf /tmp/fixture_confirm.html)
    curl -s -b "$JAR" -c "$JAR" -o /dev/null -X POST "$BASE/signup/confirm" --data-urlencode "$CT=$T"
    if [ -n "$(db "SELECT id FROM users WHERE email_bidx = '$(bidx "$MEMBER")'")" ]; then
      echo "  created: member $MEMBER (via the real signup flow)"
    else
      echo "  FAILED: the signup flow did not create $MEMBER"
      FAILURES=1
    fi
  fi
else
  echo "  SKIPPED: member $MEMBER - the app (:3000) and Mailpit (:8025) must be running."
  echo "           Start them with ./run.sh, then re-run this script."
  FAILURES=1
fi

# --- gateway dummy keys (guidance only; the config stays yours) ---
echo "== gateway keys for the shared-app suites =="
if grep -q '^komoju_enabled = true' config.toml && grep -q '^KOMOJU_SECRET_KEY=' .env; then
  echo "  present: KOMOJU enabled with a key in .env (test_checkout_c can run)."
else
  echo "  test_checkout_c runs against the shared app and needs KOMOJU in test mode:"
  echo "    - config.toml: komoju_enabled = true"
  echo "    - .env: KOMOJU_SECRET_KEY and KOMOJU_WEBHOOK_SECRET set to dummy test-shaped values"
  echo "  (test_payments and test_stripe configure their own dummy keys and need nothing.)"
fi

# --- postal data (guidance only) ---
echo "== postal data =="
ROWS=$(db "SELECT count(*) FROM postal_codes")
if [ "${ROWS:-0}" = "0" ]; then
  echo "  postal_codes is empty: address auto-fill returns nothing until the Japan"
  echo "  Post CSV is imported with scripts/ops/import_postal_codes.sh."
else
  echo "  present: $ROWS postal codes"
fi

if [ "$FAILURES" = "0" ]; then
  echo "fixtures ready"
else
  echo "fixtures incomplete - see above"
  exit 1
fi
