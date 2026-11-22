#!/usr/bin/env bash
# Maintains the unit-test specification (単体試験仕様書):
# docs/development/UNIT_TEST_SPEC.md
#
# The MD is the inventory of every unit test in the workspace —
# one row per test with its target, viewpoint （試験観点） and
# expected result （期待結果）. This script keeps that inventory
# from drifting away from the code:
#
#   generate  Rebuild the inventory table from
#             `cargo test --workspace -- --list`. Rows for tests
#             that already exist keep their ID and their filled-in
#             対象 / 観点 / 期待結果 / 結果 (keyed by the full test
#             path). New tests are appended with fresh IDs and
#             （未記入） markers in the 観点 / 期待結果 columns.
#             IDs are never reused: the highest ID ever issued is
#             recorded in the MD itself (MAX_UT_ID marker), so a
#             deleted test's ID stays retired.
#   run       Execute the full `cargo test --workspace` run,
#             write each test's result (Pass / Fail / Ignored)
#             into the inventory, and append a dated execution
#             record (date, total, passed, failed). Exits
#             non-zero when any test fails.
#   check     The anti-drift gate: the MD's case rows must match
#             `cargo test -- --list` 1:1 by full test path, and
#             no row may still carry the （未記入） marker. Run
#             this after adding or removing a test.
#
# 単体試験仕様書（docs/development/UNIT_TEST_SPEC.md）の保守用。
# テストを追加したら generate → 観点・期待結果を記入 → check、
# 実施したら run で結果を記録する。
#
# Usage: scripts/ops/unit_test_spec.sh {generate|run|check}
set -euo pipefail

cd "$(dirname "$0")/../.."
SPEC="docs/development/UNIT_TEST_SPEC.md"
UNFILLED="（未記入）"
NOT_RUN="未実施"

if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
    # scripts/check.sh has the same requirement; mirror it quietly.
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
fi

list_tests() {
    cargo test --workspace -- --list 2>/dev/null \
        | sed -n 's/: test$//p' \
        | awk '!seen[$0]++'
}

group_of() {
    # $1 = full test path -> "<crate>|<module group>"
    local p="$1" g
    if [[ "$p" == *" - "* ]]; then
        g="${p#* - }"
        echo "app|${g%::*}"
        return
    fi
    g="${p%::*}"
    g="${g%::tests}"
    g="${g%::safe_next_*_tests}"
    if [[ "$p" == config::* ]]; then
        echo "configuration|$g"
    else
        echo "app|$g"
    fi
}

target_of() {
    # Default 対象 for a newly discovered test: its module group.
    group_of "$1" | cut -d'|' -f2
}

ensure_skeleton() {
    if [ -f "$SPEC" ]; then
        return
    fi
    mkdir -p "$(dirname "$SPEC")"
    cat > "$SPEC" <<'MD_EOF'
# 単体試験仕様書（UNIT_TEST_SPEC）

本書は、本プロジェクトの単体試験（Rust のユニットテスト）の一覧と実施記録である。
対象・試験観点・期待結果をテストごとに明記し、コードとの乖離を防ぐために
保守スクリプトで管理する。

## 運用方法

保守スクリプト: `scripts/ops/unit_test_spec.sh`

| コマンド | 内容 |
|---|---|
| `generate` | `cargo test --workspace -- --list` から一覧表を再構築する。既存ケースの ID と記入済みの内容は保持され、新規テストは新しい ID と（未記入）マーカー付きで追加される |
| `run` | `cargo test --workspace` を全件実行し、各ケースの結果と日付入りの実施記録を本書へ書き込む |
| `check` | 本書のケース一覧が `cargo test -- --list` と 1:1 で一致し、（未記入）が残っていないことを検証する（テスト追加後に必ず実行する） |

手順: テストを追加したら `generate` → 新規行の対象・観点・期待結果を記入 → `check` で検証。
試験を実施したら `run` で結果を記録する。

## 試験 ID の規約

- ID は `UT-001` 形式で、一覧への登場順に採番する。
- ID はケースの恒久的な識別子であり、**再利用しない**。テストが削除されても
  その ID は欠番のまま残る（採番済みの最大 ID は一覧の MAX_UT_ID マーカーで管理する）。
- 表はクレート → モジュールの順にグループ化している。

## ケース一覧

<!-- MAX_UT_ID: 0 -->
<!-- INVENTORY:BEGIN -->
<!-- INVENTORY:END -->

## 実施記録

`run` のたびに1行追加される。

| 実施日 | 対象件数 | 合格 | 不合格 | 備考 |
|---|---|---|---|---|
<!-- EXECUTION:BEGIN -->
<!-- EXECUTION:END -->
MD_EOF
    echo "Created skeleton: $SPEC"
}

# Reads the current MD and emits TSV rows:
#   path <TAB> id <TAB> target <TAB> viewpoint <TAB> expected <TAB> result
read_rows() {
    [ -f "$SPEC" ] || return 0
    awk -F'|' '
        /^\| UT-[0-9]+ \|/ {
            for (i = 2; i <= 7; i++) {
                gsub(/^[ \t]+|[ \t]+$/, "", $i)
            }
            print $3 "\t" $2 "\t" $4 "\t" $5 "\t" $6 "\t" $7
        }' "$SPEC"
}

max_id() {
    local m=0 n
    n="$(sed -n 's/.*<!-- MAX_UT_ID: \([0-9]*\) -->.*/\1/p' "$SPEC" 2>/dev/null | head -1)"
    [ -n "$n" ] && m="$n"
    local r
    r="$(read_rows | cut -f2 | sed 's/UT-//' | sort -n | tail -1)"
    [ -n "$r" ] && [ "$r" -gt "$m" ] && m="$r"
    echo "$m"
}

cmd_generate() {
    ensure_skeleton
    local tmp
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' RETURN

    list_tests > "$tmp/list.txt"
    read_rows > "$tmp/rows.tsv"
    local next_id
    next_id="$(max_id)"

    : > "$tmp/inventory.md"
    local current_group="" path gid gcrate gmod id target vp exp res
    while IFS= read -r path; do
        gid="$(group_of "$path")"
        if [ "$gid" != "$current_group" ]; then
            current_group="$gid"
            gcrate="${gid%%|*}"
            gmod="${gid#*|}"
            {
                echo ""
                echo "### $gcrate / $gmod"
                echo ""
                echo "| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |"
                echo "|---|---|---|---|---|---|"
            } >> "$tmp/inventory.md"
        fi
        local existing
        existing="$(awk -F'\t' -v p="$path" '$1 == p {print; exit}' "$tmp/rows.tsv")"
        if [ -n "$existing" ]; then
            IFS=$'\t' read -r _ id target vp exp res <<< "$existing"
        else
            next_id=$((next_id + 1))
            id="$(printf 'UT-%03d' "$next_id")"
            target="$(target_of "$path")"
            vp="$UNFILLED"
            exp="$UNFILLED"
            res="$NOT_RUN"
        fi
        echo "| $id | $path | $target | $vp | $exp | $res |" >> "$tmp/inventory.md"
    done < "$tmp/list.txt"

    # Splice the new inventory (and the MAX_UT_ID marker) into the MD.
    awk -v inv="$tmp/inventory.md" -v maxid="$next_id" '
        /<!-- MAX_UT_ID:/ { print "<!-- MAX_UT_ID: " maxid " -->"; next }
        /<!-- INVENTORY:BEGIN -->/ {
            print
            while ((getline line < inv) > 0) print line
            skip = 1
            next
        }
        /<!-- INVENTORY:END -->/ { skip = 0; print; next }
        !skip { print }
    ' "$SPEC" > "$tmp/new.md"
    mv "$tmp/new.md" "$SPEC"
    echo "generate: $(wc -l < "$tmp/list.txt" | tr -d ' ') cases written to $SPEC"
}

cmd_check() {
    if [ ! -f "$SPEC" ]; then
        echo "check: $SPEC does not exist — run generate first." >&2
        exit 1
    fi
    local tmp
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' RETURN

    list_tests | sort > "$tmp/listed.txt"
    read_rows | cut -f1 | sort > "$tmp/in_md.txt"

    local fail=0
    if ! cmp -s "$tmp/listed.txt" "$tmp/in_md.txt"; then
        fail=1
        echo "check: inventory drift detected" >&2
        echo "--- in cargo test -- --list but missing from the MD:" >&2
        comm -23 "$tmp/listed.txt" "$tmp/in_md.txt" >&2
        echo "--- in the MD but no longer listed by cargo:" >&2
        comm -13 "$tmp/listed.txt" "$tmp/in_md.txt" >&2
    fi
    local unfilled
    unfilled="$(read_rows | awk -F'\t' -v m="$UNFILLED" '$4 == m || $5 == m {print $2 "  " $1}')"
    if [ -n "$unfilled" ]; then
        fail=1
        echo "check: cases still carrying $UNFILLED （未記入）:" >&2
        echo "$unfilled" >&2
    fi
    if [ "$fail" -ne 0 ]; then
        exit 1
    fi
    echo "check: OK — $(wc -l < "$tmp/listed.txt" | tr -d ' ') cases, inventory matches cargo test -- --list 1:1, no $UNFILLED left"
}

cmd_run() {
    if [ ! -f "$SPEC" ]; then
        echo "run: $SPEC does not exist — run generate first." >&2
        exit 1
    fi
    local tmp
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' RETURN

    list_tests | sort > "$tmp/listed.txt"
    local listed_count
    listed_count="$(wc -l < "$tmp/listed.txt" | tr -d ' ')"

    echo "run: cargo test --workspace (log: $tmp/cargo_test.log)"
    local status=0
    cargo test --workspace > "$tmp/cargo_test.log" 2>&1 || status=$?
    cp "$tmp/cargo_test.log" /tmp/unit_test_spec_last_run.log

    # Per-test results: "test <path> ... ok|FAILED|ignored"
    sed -n 's/^test \(.*\) \.\.\. \(ok\|FAILED\|ignored\)$/\1\t\2/p' \
        "$tmp/cargo_test.log" | sort > "$tmp/results.tsv"

    local passed failed ignored
    passed="$(awk -F'\t' '$2 == "ok"' "$tmp/results.tsv" | wc -l | tr -d ' ')"
    failed="$(awk -F'\t' '$2 == "FAILED"' "$tmp/results.tsv" | wc -l | tr -d ' ')"
    ignored="$(awk -F'\t' '$2 == "ignored"' "$tmp/results.tsv" | wc -l | tr -d ' ')"
    local executed=$((passed + failed + ignored))

    # Write per-case results into the inventory rows.
    awk -F'|' -v resfile="$tmp/results.tsv" '
        BEGIN {
            while ((getline line < resfile) > 0) {
                split(line, a, "\t")
                r[a[1]] = (a[2] == "ok" ? "Pass" : (a[2] == "FAILED" ? "Fail" : "Ignored"))
            }
        }
        /^\| UT-[0-9]+ \|/ {
            p = $3
            gsub(/^[ \t]+|[ \t]+$/, "", p)
            if (p in r) {
                $7 = " " r[p] " "
                line = $1
                for (i = 2; i <= 7; i++) line = line "|" $i
                print line "|"
                next
            }
        }
        { print }
    ' "$SPEC" > "$tmp/new.md"
    mv "$tmp/new.md" "$SPEC"

    # Append the dated execution record.
    local today note
    today="$(date +%Y-%m-%d)"
    if [ "$failed" -eq 0 ] && [ "$executed" -eq "$listed_count" ]; then
        note="全件実施"
    else
        note="要確認: listed=$listed_count executed=$executed"
    fi
    awk -v rec="| $today | $listed_count | $passed | $failed | $note |" '
        /<!-- EXECUTION:END -->/ { print rec; print; next }
        { print }
    ' "$SPEC" > "$tmp/new2.md"
    mv "$tmp/new2.md" "$SPEC"

    echo "run: total=$listed_count executed=$executed passed=$passed failed=$failed ignored=$ignored"
    if [ "$failed" -ne 0 ] || [ "$status" -ne 0 ]; then
        echo "run: FAILURES present (see /tmp/unit_test_spec_last_run.log)" >&2
        exit 1
    fi
    if [ "$executed" -ne "$listed_count" ]; then
        echo "run: executed count differs from the listed count" >&2
        exit 1
    fi
}

case "${1:-}" in
    generate) cmd_generate ;;
    run) cmd_run ;;
    check) cmd_check ;;
    *)
        echo "Usage: $0 {generate|run|check}" >&2
        echo "  generate  単体試験仕様書の一覧表を cargo test -- --list から再構築する" >&2
        echo "  run       cargo test を全件実行し、結果と実施記録を仕様書へ書き込む" >&2
        echo "  check     仕様書の一覧がコードと 1:1 で一致し、（未記入）が無いか検証する" >&2
        exit 2
        ;;
esac
