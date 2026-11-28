#!/bin/bash
# Verifies the postal-code autofill: the module is served
# and calls the store's own lookup API (no outside service),
# the storefront layout loads it, the CSP needs no outside
# host for it, and the endpoint answers from the
# postal_codes table — fixture rows under fake codes are
# inserted and removed by this suite, so it passes whether
# or not the real Japan Post data has been imported yet.
# The fill behaviour itself is browser-only — fetch under
# the real CSP is invisible to curl — so it is verified in
# a real browser, not here.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"

PSQL() { docker exec -i agentic-db psql -U agentic -d agentic_commerce -t -A "$@"; }
cleanup_fixtures() {
  PSQL -c "DELETE FROM postal_codes WHERE postal_code IN ('0000000', '0000001')" >/dev/null 2>&1
}
trap cleanup_fixtures EXIT
cleanup_fixtures
PSQL -c "INSERT INTO postal_codes (postal_code, prefecture, city, town) VALUES
  ('0000000', 'テスト県', 'テスト市', '中央町'),
  ('0000000', 'テスト県', 'テスト市', '西町'),
  ('0000001', 'テスト県', 'テスト市', '')" >/dev/null

echo "-- the module"
CODE=$(curl -s -o /tmp/postal_js.txt -w "%{http_code}" $BASE/static/js/postal-autofill.js)
[ "$CODE" = "200" ] && ok "postal-autofill.js is served" || ng "postal-autofill.js is served ($CODE)"
grep -q "/api/postal-codes/" /tmp/postal_js.txt && ok "module calls the store's own lookup API" || ng "module calls the store's own lookup API"
grep -q "zipcloud" /tmp/postal_js.txt && ng "module names no outside service" || ok "module names no outside service"
grep -q 'input\[name="postal_code"\]' /tmp/postal_js.txt && ok "module wires onto postal_code fields" || ng "module wires onto postal_code fields"

echo "-- the layout loads it on form pages"
curl -s $BASE/ -o /tmp/postal_home.html
grep -q "postal-autofill.js?v=" /tmp/postal_home.html && ok "layout loads the module with the asset fingerprint" || ng "layout loads the module with the asset fingerprint"

echo "-- the CSP needs no outside host for the lookup"
CSP=$(curl -sI $BASE/ | grep -i "^content-security-policy:")
echo "$CSP" | grep -q "connect-src 'self';" && ok "CSP connect-src is self only" || ng "CSP connect-src is self only"
echo "$CSP" | grep -q "zipcloud" && ng "CSP names no outside host" || ok "CSP names no outside host"
echo "$CSP" | grep -q "script-src 'self'" && ok "CSP script-src stays self-only" || ng "CSP script-src stays self-only"

echo "-- the lookup endpoint"
BODY=$(curl -s $BASE/api/postal-codes/0000000)
echo "$BODY" | grep -q '"prefecture":"テスト県"' && echo "$BODY" | grep -q '"town":"中央町"' \
  && echo "$BODY" | grep -q '"town":"西町"' \
  && ok "multi-town code returns every town" || ng "multi-town code returns every town"
BODY=$(curl -s $BASE/api/postal-codes/0000001)
echo "$BODY" | grep -q '"city":"テスト市"' && echo "$BODY" | grep -q '"town":""' \
  && ok "city-level code returns an empty town" || ng "city-level code returns an empty town"
CODE=$(curl -s -o /tmp/postal_404.txt -w "%{http_code}" $BASE/api/postal-codes/0000002)
[ "$CODE" = "404" ] && grep -q '"error":"address_not_found"' /tmp/postal_404.txt \
  && ok "unknown code answers 404 address_not_found" || ng "unknown code answers 404 address_not_found ($CODE)"
CODE=$(curl -s -o /tmp/postal_400.txt -w "%{http_code}" $BASE/api/postal-codes/123)
[ "$CODE" = "400" ] && grep -q '"error":"invalid_postal_code"' /tmp/postal_400.txt \
  && ok "malformed code answers 400 invalid_postal_code" || ng "malformed code answers 400 invalid_postal_code ($CODE)"

result
