#!/bin/bash
# HTTP security headers and session-cookie flags (overnight
# review T-2): the header layer and the cookie attributes are
# security behaviour no other suite asserted, so a middleware
# reorder or a session-layer change could strip them silently.
# Attribute checks match flag NAMES only — cookie values are
# credentials and are never printed.
BASE=http://127.0.0.1:3000
# Shared harness: ok/ng, csrf() and result() live in lib.sh (ST-6).
. "$(dirname "$0")/lib.sh"

echo "-- page security headers"
H=$(curl -s -D - -o /dev/null $BASE/)
echo "$H" | grep -qi "^content-security-policy:" && ok "CSP header present" || ng "CSP header present"
echo "$H" | grep -qi "default-src 'self'" && ok "CSP default-src is 'self'" || ng "CSP default-src is 'self'"
echo "$H" | grep -qi "form-action 'self' https://komoju.com" && ok "CSP form-action allows the KOMOJU redirect" || ng "CSP form-action allows the KOMOJU redirect"
echo "$H" | grep -qi "frame-ancestors 'none'" && ok "CSP frame-ancestors is 'none'" || ng "CSP frame-ancestors is 'none'"
echo "$H" | grep -qi "^x-content-type-options: nosniff" && ok "X-Content-Type-Options nosniff" || ng "X-Content-Type-Options nosniff"
echo "$H" | grep -qi "^x-frame-options: DENY" && ok "X-Frame-Options DENY" || ng "X-Frame-Options DENY"
echo "$H" | grep -qi "^referrer-policy: same-origin" && ok "Referrer-Policy same-origin" || ng "Referrer-Policy same-origin"
echo "$H" | grep -qi "^x-request-id:" && ok "x-request-id is issued" || ng "x-request-id is issued"

echo "-- session cookie flags"
C=$(curl -s -D - -o /dev/null $BASE/login | grep -i "^set-cookie:")
echo "$C" | grep -qi "httponly" && ok "member cookie is HttpOnly" || ng "member cookie is HttpOnly"
echo "$C" | grep -qi "samesite=lax" && ok "member cookie is SameSite=Lax" || ng "member cookie is SameSite=Lax"
A=$(curl -s -D - -o /dev/null $BASE/admin/login | grep -i "^set-cookie:")
echo "$A" | grep -qi "admin_session" && ok "admin cookie is named admin_session" || ng "admin cookie is named admin_session"
echo "$A" | grep -qi "path=/admin" && ok "admin cookie is scoped to /admin" || ng "admin cookie is scoped to /admin"
echo "$A" | grep -qi "httponly" && ok "admin cookie is HttpOnly" || ng "admin cookie is HttpOnly"

echo "-- fingerprinted asset caching"
FP=$(curl -s $BASE/ | grep -o 'style.css?v=[a-f0-9]*' | head -1 | sed 's/.*v=//')
CC=$(curl -s -D - -o /dev/null "$BASE/static/css/style.css?v=$FP" | grep -i "^cache-control:")
echo "$CC" | grep -q "immutable" && ok "real fingerprint earns immutable caching" || ng "real fingerprint earns immutable caching"
CC2=$(curl -s -D - -o /dev/null "$BASE/static/css/style.css?v=0000000000000000" | grep -i "^cache-control:")
echo "$CC2" | grep -q "immutable" && ng "forged fingerprint must not earn immutable" || ok "forged fingerprint must not earn immutable"

result
