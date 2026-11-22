#!/bin/bash
# Admin two-factor verification: the
# admin surface was password-only while members had TOTP.
# Covers enrolment from the account settings page, the
# login challenge (dashboard unreachable while pending),
# a wrong code refused, the right code admitted, a
# recovery code admitted exactly once, and the disable
# ceremony returning the account to password-only login.
#
# SAFETY: the suite enrols the SHARED bootstrap admin.
# A trap clears the enrolment in SQL on ANY exit, so a
# failed run can never leave the admin TOTP-locked for
# the other suites. Run it on a freshly restarted app
# (in-memory login throttles), never back-to-back with
# test_admin.sh.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
AID=$(db "SELECT id FROM admins WHERE email = '$ADMIN_EMAIL'")
J=/tmp/at_admin.txt

clear_totp() {
  db "UPDATE admins SET totp_secret = NULL, totp_enabled_at = NULL, totp_last_step = NULL WHERE id = $AID" >/dev/null
  db "DELETE FROM admin_recovery_codes WHERE admin_id = $AID" >/dev/null
}
trap clear_totp EXIT
clear_totp

totp_now() { # base32 secret -> current 6-digit code
  python3 - "$1" <<'PYEOF'
import base64, hashlib, hmac, struct, sys, time
key = base64.b32decode(sys.argv[1])
counter = int(time.time()) // 30
digest = hmac.new(key, struct.pack(">Q", counter), hashlib.sha1).digest()
off = digest[-1] & 0x0F
code = (struct.unpack(">I", digest[off:off + 4])[0] & 0x7FFFFFFF) % 1000000
print(f"{code:06d}")
PYEOF
}

wait_fresh_code() { # secret used_code -> echoes a code from a later step
  local code
  for _ in $(seq 1 40); do
    code=$(totp_now "$1")
    [ "$code" != "$2" ] && { echo "$code"; return; }
    sleep 2
  done
  echo "$code"
}

admin_login() { # -> echoes the login POST's redirect target
  rm -f "$J"
  curl -s -c "$J" $BASE/admin/login -o /tmp/at_login.html
  local C=$(csrf /tmp/at_login.html)
  curl -s -b "$J" -c "$J" -o /dev/null -w "%{redirect_url}" -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "password=$APW" --data-urlencode "csrf_token=$C"
}

admin_logout() {
  curl -s -b "$J" $BASE/admin -o /tmp/at_dash.html
  local C=$(csrf /tmp/at_dash.html)
  curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/logout --data-urlencode "csrf_token=$C"
}

challenge_post() { # code -> response body in /tmp/at_chal.html, echoes redirect
  curl -s -b "$J" $BASE/admin/login/totp -o /tmp/at_chal.html
  local C=$(csrf /tmp/at_chal.html)
  curl -s -b "$J" -c "$J" -o /tmp/at_chal.html -w "%{redirect_url}" -X POST $BASE/admin/login/totp \
    --data-urlencode "code=$1" --data-urlencode "csrf_token=$C"
}

echo "-- enrolment: setup shows a secret, a code enables it"
OUT=$(admin_login)
case "$OUT" in *"/admin") ok "password-only login works before enrolment";; *) ng "password-only login works before enrolment ($OUT)";; esac
curl -s -b "$J" $BASE/admin/password -o /tmp/at_set.html
grep -q 'action="/admin/totp/setup"' /tmp/at_set.html && ok "settings page offers setup" || ng "settings page offers setup"
C=$(csrf /tmp/at_set.html)
curl -s -b "$J" -c "$J" -X POST $BASE/admin/totp/setup --data-urlencode "csrf_token=$C" -o /tmp/at_set.html
SECRET=$(grep -o 'totp-secret">[^<]*' /tmp/at_set.html | sed 's/totp-secret">//; s/ //g')
[ -n "$SECRET" ] && ok "setup renders a pending secret" || ng "setup renders a pending secret"
CODE=$(totp_now "$SECRET")
C=$(csrf /tmp/at_set.html)
curl -s -b "$J" -c "$J" -X POST $BASE/admin/totp/enable --data-urlencode "code=$CODE" --data-urlencode "csrf_token=$C" -o /tmp/at_rec.html
grep -q "リカバリーコード" /tmp/at_rec.html && ok "enable shows the recovery codes once" || ng "enable shows the recovery codes once"
RECOVERY=$(grep -o '<li><code>[^<]*' /tmp/at_rec.html | head -1 | sed 's/<li><code>//')
[ -n "$RECOVERY" ] && ok "a recovery code was issued" || ng "a recovery code was issued"
[ "$(db "SELECT totp_enabled_at IS NOT NULL FROM admins WHERE id = $AID")" = "t" ] && ok "enrolment persisted" || ng "enrolment persisted"

echo "-- challenge: pending is not signed in, wrong refused, right admitted"
admin_logout
OUT=$(admin_login)
case "$OUT" in *"/admin/login/totp") ok "password success defers to the challenge";; *) ng "password success defers to the challenge ($OUT)";; esac
OUT=$(curl -s -b "$J" -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/admin)
case "$OUT" in *"/admin/login") ok "dashboard refuses a pending session";; *) ng "dashboard refuses a pending session ($OUT)";; esac
REAL=$(totp_now "$SECRET")
WRONG=000000; [ "$WRONG" = "$REAL" ] && WRONG=000001
challenge_post "$WRONG" > /dev/null
grep -q "認証コードが正しくありません" /tmp/at_chal.html && ok "wrong code refused with the generic error" || ng "wrong code refused with the generic error"
OUT=$(challenge_post "$REAL")
case "$OUT" in *"/admin") ok "right code completes the login";; *) ng "right code completes the login ($OUT)";; esac
CODE=$(curl -s -b "$J" -o /dev/null -w "%{http_code}" $BASE/admin)
[ "$CODE" = "200" ] && ok "dashboard reachable after the challenge" || ng "dashboard reachable after the challenge ($CODE)"

echo "-- recovery code: admitted once, refused on reuse"
admin_logout
OUT=$(admin_login)
case "$OUT" in *"/admin/login/totp") ok "second login challenges again";; *) ng "second login challenges again ($OUT)";; esac
OUT=$(challenge_post "$RECOVERY")
case "$OUT" in *"/admin") ok "recovery code completes the login";; *) ng "recovery code completes the login ($OUT)";; esac
admin_logout
admin_login > /dev/null
challenge_post "$RECOVERY" > /dev/null
grep -q "認証コードが正しくありません" /tmp/at_chal.html && ok "the same recovery code is refused on reuse" || ng "the same recovery code is refused on reuse"
FRESH=$(wait_fresh_code "$SECRET" "$REAL")
OUT=$(challenge_post "$FRESH")
case "$OUT" in *"/admin") ok "a fresh authenticator code still admits";; *) ng "a fresh authenticator code still admits ($OUT)";; esac

echo "-- disable: password + code proof returns to password-only"
curl -s -b "$J" $BASE/admin/password -o /tmp/at_set.html
grep -q 'action="/admin/totp/disable"' /tmp/at_set.html && ok "settings page shows the disable ceremony" || ng "settings page shows the disable ceremony"
CODE=$(totp_now "$SECRET")
C=$(csrf /tmp/at_set.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/totp/disable \
  --data-urlencode "password=$APW" --data-urlencode "code=$CODE" --data-urlencode "csrf_token=$C"
[ "$(db "SELECT totp_enabled_at IS NULL AND totp_secret IS NULL FROM admins WHERE id = $AID")" = "t" ] && ok "disable cleared the enrolment" || ng "disable cleared the enrolment"
admin_logout
OUT=$(admin_login)
case "$OUT" in *"/admin") ok "login is password-only again after disable";; *) ng "login is password-only again after disable ($OUT)";; esac

result
