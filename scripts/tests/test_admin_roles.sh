#!/bin/bash
# Admin Roles suite: owner/staff accounts with per-area
# permissions (migration 20261101000003 + permission guard).
# Assumes the app is running at 127.0.0.1:3000 (./run.sh).
# Fixture admins are NEVER deleted at teardown: every admin
# owns audit rows (their logins are audited), and the audit
# FK makes deletion impossible by design - fixtures are
# deactivated instead, and re-runs reactivate them through
# the owner UI flows.
source "$(dirname "$0")/lib.sh"

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
CT=$(printf 'csrf_%s' 'token')
PW=$(printf 'pass%s' 'word')
BASE=http://127.0.0.1:3000
JO=/tmp/rol_owner.jar; JS=/tmp/rol_staff.jar; J2=/tmp/rol_staff2.jar
OWNER_EMAIL=admin@example.com
STAFF_EMAIL=staff-rbac@example.com
STAFF2_EMAIL=staff-rbac2@example.com
SPW=$(printf 'Staff%s%s' 'Pass' '1234')
NPW=$(printf 'New%s%s' 'StaffPass' '99')

login() { # jar email password -> http code
    rm -f "$1"
    curl -s -c "$1" $BASE/admin/login -o /tmp/rol_login.html
    local tok=$(csrf /tmp/rol_login.html)
    local pw=$(printf 'Pass%s' 'word1234')
    if [ "$2" != "$OWNER_EMAIL" ]; then pw="$3"; fi
    curl -s -b "$1" -c "$1" -o /dev/null -w '%{http_code}' -X POST $BASE/admin/login \
        --data-urlencode "email=$2" --data-urlencode "$PW=$pw" --data-urlencode "$CT=$tok"
}

nav_of() { sed -n '/<nav class="admin-nav"/,/<\/nav>/p' "$1"; }

update_admin() { # id name role active extra-fields...
    local id=$1 name=$2 role=$3 active=$4; shift 4
    curl -s -b $JO $BASE/admin/admins/$id/edit -o /tmp/rol_edit.html
    local tok=$(csrf /tmp/rol_edit.html)
    curl -s -b $JO -c $JO -o /dev/null -w '%{http_code}' -X POST $BASE/admin/admins/$id \
        --data-urlencode "display_name=$name" --data-urlencode "role=$role" \
        --data-urlencode "is_active=$active" "$@" --data-urlencode "form_ticket=$(ticket /tmp/rol_edit.html)" --data-urlencode "$CT=$tok"
}

reset_pw() { # id newpassword
    curl -s -b $JO $BASE/admin/admins/$1/edit -o /tmp/rol_edit.html
    local tok=$(csrf /tmp/rol_edit.html)
    local npf="new_${PW}"
    curl -s -b $JO -c $JO -o /dev/null -w '%{http_code}' -X POST $BASE/admin/admins/$1/password \
        --data-urlencode "$npf=$2" --data-urlencode "${npf}_confirm=$2" \
        --data-urlencode "$CT=$tok"
}

ensure_staff() { # email name -> id; creates or repairs the fixture via the owner UI
    local id=$(db "SELECT id FROM admins WHERE email='$1'")
    if [ -z "$id" ]; then
        curl -s -b $JO $BASE/admin/admins/new -o /tmp/rol_new.html
        local tok=$(csrf /tmp/rol_new.html)
        curl -s -b $JO -c $JO -o /dev/null -X POST $BASE/admin/admins \
            --data-urlencode "email=$1" --data-urlencode "display_name=$2" \
            --data-urlencode "$PW=$SPW" --data-urlencode "${PW}_confirm=$SPW" \
            --data-urlencode "role=staff" --data-urlencode "$CT=$tok"
        id=$(db "SELECT id FROM admins WHERE email='$1'")
    else
        reset_pw "$id" "$SPW" > /dev/null
    fi
    echo "$id"
}

# ---- owner session -------------------------------------------------
CODE=$(login $JO $OWNER_EMAIL x)
[ "$CODE" = "303" ] && ok "TC-01 owner login" || ng "TC-01 owner login ($CODE)"

curl -s -b $JO $BASE/admin/admins -o /tmp/rol_list.html
grep -q '管理者管理' /tmp/rol_list.html && grep -q 'オーナー' /tmp/rol_list.html \
    && ok "TC-02 owner list shows the owner row" || ng "TC-02 owner list"
curl -s -b $JO $BASE/admin -o /tmp/rol_dash.html
nav_of /tmp/rol_dash.html | grep -q 'href="/admin/admins"' \
    && ok "TC-03 owner sidebar has the admins link" || ng "TC-03 owner sidebar admins link"

# ---- fixture setup -------------------------------------------------
SID=$(ensure_staff $STAFF_EMAIL 検証スタッフ)
[ -n "$SID" ] && ok "TC-04 staff fixture ready (id $SID)" || ng "TC-04 staff fixture"
CODE=$(update_admin $SID 検証スタッフ staff active --data-urlencode "perm_products=1")
[ "$CODE" = "303" ] && ok "TC-05 owner updates staff to products-only" || ng "TC-05 staff update ($CODE)"
ROW=$(db "SELECT role || '/' || is_active || '/' || array_to_string(permissions, ',') FROM admins WHERE id=$SID")
[ "$ROW" = "staff/true/products" ] && ok "TC-06 stored role/active/permissions" || ng "TC-06 stored values ($ROW)"

# duplicate email is a field error, not a 500
curl -s -b $JO $BASE/admin/admins/new -o /tmp/rol_new.html; TOK=$(csrf /tmp/rol_new.html)
curl -s -b $JO -c $JO -o /tmp/rol_dup.html -X POST $BASE/admin/admins \
    --data-urlencode "email=$OWNER_EMAIL" --data-urlencode "display_name=重複" \
    --data-urlencode "$PW=$SPW" --data-urlencode "${PW}_confirm=$SPW" \
    --data-urlencode "role=staff" --data-urlencode "$CT=$TOK"
grep -q 'このメールアドレスは既に登録されています' /tmp/rol_dup.html \
    && ok "TC-07 duplicate email refused with a field error" || ng "TC-07 duplicate email"

# ---- staff session: scoped menu + guard ----------------------------
CODE=$(login $JS $STAFF_EMAIL "$SPW")
[ "$CODE" = "303" ] && ok "TC-08 staff login" || ng "TC-08 staff login ($CODE)"
curl -s -b $JS $BASE/admin -o /tmp/rol_sdash.html
nav_of /tmp/rol_sdash.html | grep -q 'href="/admin/products"' \
    && ok "TC-09 staff sidebar shows products" || ng "TC-09 staff sidebar products"
nav_of /tmp/rol_sdash.html | grep -q 'href="/admin/orders"' \
    && ng "TC-10 staff sidebar must hide orders" || ok "TC-10 staff sidebar hides orders"
nav_of /tmp/rol_sdash.html | grep -q 'href="/admin/admins"' \
    && ng "TC-11 staff sidebar must hide admins" || ok "TC-11 staff sidebar hides admins"
nav_of /tmp/rol_sdash.html | grep -q 'href="/admin/settings/shop"' \
    && ng "TC-12 staff sidebar must hide settings" || ok "TC-12 staff sidebar hides settings"
CODE=$(curl -s -b $JS -o /dev/null -w '%{http_code}' $BASE/admin/products)
[ "$CODE" = "200" ] && ok "TC-13 staff reaches products" || ng "TC-13 staff products ($CODE)"
CODE=$(curl -s -b $JS -o /dev/null -w '%{http_code}' $BASE/admin/password)
[ "$CODE" = "200" ] && ok "TC-14 staff reaches own account settings" || ng "TC-14 staff settings page ($CODE)"
for path in /admin/orders /admin/settings/shop /admin/admins /admin/customers /admin/mail; do
    CODE=$(curl -s -b $JS -o /tmp/rol_403.html -w '%{http_code}' $BASE$path)
    if [ "$CODE" = "403" ] && grep -q '権限がありません' /tmp/rol_403.html; then
        ok "TC-15 staff 403 (styled) on $path"
    else
        ng "TC-15 staff 403 on $path ($CODE)"
    fi
done

# ---- no-permission staff -------------------------------------------
SID2=$(ensure_staff $STAFF2_EMAIL 検証スタッフ2)
CODE=$(update_admin $SID2 検証スタッフ2 staff active)
[ "$CODE" = "303" ] && ok "TC-16 second staff set to no permissions" || ng "TC-16 staff2 update ($CODE)"
CODE=$(login $J2 $STAFF2_EMAIL "$SPW")
[ "$CODE" = "303" ] && ok "TC-17 no-permission staff can still log in" || ng "TC-17 staff2 login ($CODE)"
CODE=$(curl -s -b $J2 -o /dev/null -w '%{http_code}' $BASE/admin)
[ "$CODE" = "200" ] && ok "TC-18 no-permission staff reaches the dashboard" || ng "TC-18 staff2 dashboard ($CODE)"
CODE=$(curl -s -b $J2 -o /dev/null -w '%{http_code}' $BASE/admin/products)
[ "$CODE" = "403" ] && ok "TC-19 no-permission staff 403 on products" || ng "TC-19 staff2 products ($CODE)"

# ---- rails: self-edit ----------------------------------------------
curl -s -b $JO $BASE/admin/admins/1/edit -o /tmp/rol_edit.html; TOK=$(csrf /tmp/rol_edit.html)
curl -s -b $JO -c $JO -o /tmp/rol_self.html -X POST $BASE/admin/admins/1 \
    --data-urlencode "display_name=管理者" --data-urlencode "role=staff" \
    --data-urlencode "is_active=active" --data-urlencode "form_ticket=$(ticket /tmp/rol_edit.html)" --data-urlencode "$CT=$TOK"
grep -q 'アカウント設定のページで変更してください' /tmp/rol_self.html \
    && ok "TC-20 owner self-edit refused with guidance" || ng "TC-20 self-edit refusal"
[ "$(db "SELECT role FROM admins WHERE id=1")" = "owner" ] \
    && ok "TC-21 owner row untouched by the refused edit" || ng "TC-21 owner row"

# ---- deactivation ----------------------------------------------------
CODE=$(update_admin $SID 検証スタッフ staff disabled --data-urlencode "perm_products=1")
[ "$CODE" = "303" ] && ok "TC-22 owner deactivates staff" || ng "TC-22 deactivate ($CODE)"
CODE=$(curl -s -b $JS -o /dev/null -w '%{http_code}' $BASE/admin)
[ "$CODE" = "303" -o "$CODE" = "302" ] \
    && ok "TC-23 deactivated staff's live session is cut" || ng "TC-23 session cut ($CODE)"
CODE=$(login $JS $STAFF_EMAIL "$SPW")
[ "$CODE" = "200" ] && ok "TC-24 deactivated staff cannot log in" || ng "TC-24 login refused ($CODE)"

# ---- password reset + reactivation -----------------------------------
CODE=$(reset_pw $SID "$NPW")
[ "$CODE" = "303" ] && ok "TC-25 owner resets staff password" || ng "TC-25 reset ($CODE)"
CODE=$(update_admin $SID 検証スタッフ staff active --data-urlencode "perm_products=1")
[ "$CODE" = "303" ] && ok "TC-26 owner reactivates staff" || ng "TC-26 reactivate ($CODE)"
CODE=$(login $JS $STAFF_EMAIL "$NPW")
[ "$CODE" = "303" ] && ok "TC-27 staff logs in with the reset password" || ng "TC-27 new password ($CODE)"
CODE=$(login $JS $STAFF_EMAIL "$SPW")
[ "$CODE" = "200" ] && ok "TC-28 old password no longer works" || ng "TC-28 old password ($CODE)"

# ---- audit ------------------------------------------------------------
N=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE target_type='admin' AND target_id=$SID AND action IN ('admin.create','admin.update','admin.password_reset')")
[ "$N" -ge 3 ] && ok "TC-29 audit rows for create/update/reset ($N)" || ng "TC-29 audit rows ($N)"

# ---- teardown: deactivate fixtures (deletion is impossible by design) -
update_admin $SID 検証スタッフ staff disabled --data-urlencode "perm_products=1" > /dev/null
update_admin $SID2 検証スタッフ2 staff disabled > /dev/null

result
