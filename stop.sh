#!/usr/bin/env bash
# stop.sh - Stop the Kaimoru server.
#
# Usage:
#   ./stop.sh        Stop the server only. The database container
#                        keeps running, so the next start is fast.
#   ./stop.sh db     Also stop the PostgreSQL container.

set -e
cd "$(dirname "$0")"

# fuser finds whatever process is listening on port 3000 and stops it.
# It exits quietly when nothing is listening.
if fuser 3000/tcp >/dev/null 2>&1; then
    fuser -k 3000/tcp >/dev/null 2>&1 || true
    sleep 1
    echo "Server stopped."
else
    echo "Server was not running."
fi

# No process hunting beyond the port: when the server binary dies,
# the cargo run that launched it returns on its own.

if [ "${1:-}" = "db" ]; then
    echo "Stopping the database container..."
    docker compose stop
    echo "Database stopped. Data is kept in the Docker volume."
fi

echo "Done. Start again with: ./run.sh"
