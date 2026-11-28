#!/usr/bin/env bash
# Backs up the development database and PROVES the backup by
# restoring it into a scratch database and comparing row counts.
#
# The ritual this automates (AGENTS.md): before any
# database-touching rebuild or reset, a dump outside the repo
# that has been restore-proven is the only acceptable backup —
# an unrestored dump is a hope, not a backup.
#
# Usage: scripts/ops/backup_db.sh
# Env overrides: CONTAINER (agentic-db), DB_USER (agentic),
# DB_NAME (agentic_commerce), BACKUP_DIR (~/backups/agentic-commerce)
set -euo pipefail

CONTAINER="${CONTAINER:-agentic-db}"
DB_USER="${DB_USER:-agentic}"
DB_NAME="${DB_NAME:-agentic_commerce}"
BACKUP_DIR="${BACKUP_DIR:-$HOME/backups/agentic-commerce}"
SCRATCH_DB="backup_restore_check"
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT="$BACKUP_DIR/${DB_NAME}-${STAMP}.sql"

psql_in() { docker exec -i "$CONTAINER" psql -U "$DB_USER" "$@"; }

mkdir -p "$BACKUP_DIR"

echo "Dumping $DB_NAME from container $CONTAINER ..."
docker exec "$CONTAINER" pg_dump -U "$DB_USER" -d "$DB_NAME" > "$OUT"
echo "Wrote $OUT ($(stat -c%s "$OUT") bytes)"

cleanup() {
    psql_in -d postgres -q -c "DROP DATABASE IF EXISTS $SCRATCH_DB" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "Restoring into scratch database $SCRATCH_DB ..."
cleanup
psql_in -d postgres -q -c "CREATE DATABASE $SCRATCH_DB"
psql_in -d "$SCRATCH_DB" -q -v ON_ERROR_STOP=1 < "$OUT" > /dev/null

echo "Comparing row counts (live vs restored):"
fail=0
for table in users orders products reviews user_coupons; do
    live="$(psql_in -d "$DB_NAME" -t -A -c "SELECT count(*) FROM $table")"
    copy="$(psql_in -d "$SCRATCH_DB" -t -A -c "SELECT count(*) FROM $table")"
    if [ "$live" = "$copy" ]; then
        echo "  OK  $table: $live"
    else
        echo "  MISMATCH  $table: live=$live restored=$copy"
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    echo "BACKUP NOT PROVEN — treat $OUT as unusable." >&2
    exit 1
fi
echo "Backup proven: $OUT"
