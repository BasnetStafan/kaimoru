#!/usr/bin/env bash
# Loads Japan Post postal code data into the postal_codes
# table: cleanses this month's CSV (only the needed columns
# and rows survive), stages it, and swaps it in only after
# the staged data validates — a bad file never takes the
# working data down with it.
#
# The dataset is NOT bundled with the repository (it is large
# and re-published monthly by Japan Post). Download the
# nationwide CSV (KEN_ALL, Shift_JIS or UTF-8 edition) from
# https://www.post.japanpost.jp/zipcode/download.html
#
# Monthly usage:
#   1. Download the new CSV (or its zip) from Japan Post.
#   2. Put it in ~/postal-data/ (or pass its path).
#   3. Run this script. Last month's file is simply replaced
#      by the new import; there is no diff to apply by hand.
#
# Usage: scripts/ops/import_postal_codes.sh [CSV-or-ZIP]
#   With no argument, the newest *.csv / *.zip in POSTAL_DIR
#   is used.
# Env overrides: CONTAINER (agentic-db), DB_USER (agentic),
# DB_NAME (agentic_commerce), POSTAL_DIR (~/postal-data)
set -euo pipefail

CONTAINER="${CONTAINER:-agentic-db}"
DB_USER="${DB_USER:-agentic}"
DB_NAME="${DB_NAME:-agentic_commerce}"
POSTAL_DIR="${POSTAL_DIR:-$HOME/postal-data}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

psql_in() { docker exec -i "$CONTAINER" psql -U "$DB_USER" "$@"; }

# --- find this month's file --------------------------------
INPUT="${1:-}"
if [ -z "$INPUT" ]; then
    INPUT="$(ls -t "$POSTAL_DIR"/*.csv "$POSTAL_DIR"/*.CSV \
        "$POSTAL_DIR"/*.zip "$POSTAL_DIR"/*.ZIP 2>/dev/null | head -1 || true)"
fi
if [ -z "$INPUT" ] || [ ! -f "$INPUT" ]; then
    echo "No input file. Download the postal CSV into $POSTAL_DIR" >&2
    echo "or pass its path: scripts/ops/import_postal_codes.sh <file>" >&2
    exit 1
fi
echo "Input: $INPUT"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# --- unpack if zipped ---------------------------------------
SOURCE_CSV="$INPUT"
case "$INPUT" in
    *.zip | *.ZIP)
        unzip -o -q "$INPUT" -d "$WORK/unzipped"
        SOURCE_CSV="$(find "$WORK/unzipped" -iname '*.csv' | head -1)"
        if [ -z "$SOURCE_CSV" ]; then
            echo "The zip contains no CSV file." >&2
            exit 1
        fi
        ;;
esac

# --- cleanse -------------------------------------------------
CLEAN_CSV="$WORK/postal_codes_clean.csv"
python3 "$SCRIPT_DIR/cleanse_postal_csv.py" "$SOURCE_CSV" "$CLEAN_CSV"
ROWS=$(( $(wc -l < "$CLEAN_CSV") - 1 ))
if [ "$ROWS" -le 0 ]; then
    echo "Cleansing produced no rows — import aborted, live data untouched." >&2
    exit 1
fi
if [ "$ROWS" -lt 100000 ]; then
    echo "WARNING: only $ROWS rows — a partial (single-prefecture?) file"
    echo "would REPLACE the national data. Continuing in 5 seconds;"
    echo "press Ctrl-C to abort."
    sleep 5
fi

# --- stage ---------------------------------------------------
# The stage table is all-nullable on purpose: COPY reads an
# empty CSV field as NULL, and the swap below restores the
# empty town (a city-level code) with COALESCE.
psql_in -d "$DB_NAME" -q -v ON_ERROR_STOP=1 \
    -c "DROP TABLE IF EXISTS postal_codes_stage" \
    -c "CREATE TABLE postal_codes_stage (postal_code TEXT, prefecture TEXT, city TEXT, town TEXT)"
docker exec -i "$CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -q -v ON_ERROR_STOP=1 \
    -c "\copy postal_codes_stage FROM STDIN WITH (FORMAT csv, HEADER true)" \
    < "$CLEAN_CSV"
STAGED="$(psql_in -d "$DB_NAME" -t -A -c "SELECT count(*) FROM postal_codes_stage")"
if [ "$STAGED" != "$ROWS" ]; then
    echo "Staged $STAGED rows but cleansed $ROWS — import aborted." >&2
    psql_in -d "$DB_NAME" -q -c "DROP TABLE IF EXISTS postal_codes_stage" || true
    exit 1
fi
echo "Staged: $STAGED rows"

# --- swap (one transaction: the table is never half-new) -----
psql_in -d "$DB_NAME" -q -v ON_ERROR_STOP=1 <<'SQL'
BEGIN;
TRUNCATE postal_codes;
INSERT INTO postal_codes
    SELECT postal_code, prefecture, city, COALESCE(town, '')
    FROM postal_codes_stage;
COMMIT;
DROP TABLE postal_codes_stage;
SQL

TOTAL="$(psql_in -d "$DB_NAME" -t -A -c "SELECT count(*) FROM postal_codes")"
echo "Imported: $TOTAL rows into postal_codes ($DB_NAME)"
echo "Spot check — 1000001:"
psql_in -d "$DB_NAME" -t -A -F ' / ' \
    -c "SELECT prefecture, city, town FROM postal_codes WHERE postal_code = '1000001' ORDER BY town" \
    || true
