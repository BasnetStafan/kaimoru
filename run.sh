#!/usr/bin/env bash
# run.sh - Run Kaimoru locally and open it in the browser.
#
# What it does:
#   0. Starts Ollama (the shopping agent's LLM) when it is not running.
#   1. Starts the PostgreSQL container (docker compose up -d).
#   2. Waits until the database accepts connections.
#   3. Builds and starts the server on http://localhost:3000
#      (migrations and test data are applied automatically at startup).
#   4. Opens the top page in your Windows default browser.
#
# Stop with Ctrl+C. The database container keeps running afterwards;
# stop it with: docker compose stop

set -e
cd "$(dirname "$0")"

# Print the URLs needed while the app is running.
# Open a URL in the developer's browser, per platform. Every
# candidate is guarded and silenced: on a machine without any of
# them (a headless server, a container) this simply does nothing.
open_browser() {
    if command -v cmd.exe >/dev/null 2>&1; then
        cmd.exe /c start "$1" >/dev/null 2>&1 || true
    elif command -v open >/dev/null 2>&1; then
        open "$1" >/dev/null 2>&1 || true
    elif command -v xdg-open >/dev/null 2>&1; then
        xdg-open "$1" >/dev/null 2>&1 || true
    fi
}

print_links() {
    echo "Links:"
    echo "  App:     http://localhost:3000"
    echo "  Mailpit: http://localhost:8025  (local email inbox - sign-up verification codes arrive here)"
}

# The shopping agent's LLM is a separate Ollama process (manual install
# at ~/ollama, not on PATH, no service). The app only falls back to a
# polite error when the endpoint is unreachable, so a dead Ollama
# otherwise goes unnoticed until someone chats. Start it when it is not
# answering; when .env carries LLM_API_KEY the chat uses the cloud
# endpoint instead and Ollama is not needed.
ensure_ollama() {
    if [ -f .env ] && grep -q '^LLM_API_KEY=.' .env; then
        echo "==> LLM_API_KEY is set in .env - agent chat uses the cloud endpoint, Ollama not needed"
        return 0
    fi
    if curl -s -o /dev/null -m 2 http://127.0.0.1:11434/api/tags 2>/dev/null; then
        echo "==> Ollama is already running"
        return 0
    fi
    local ollama_bin="$HOME/ollama/bin/ollama"
    if [ ! -x "$ollama_bin" ]; then
        echo "run.sh: Ollama not found at $ollama_bin - agent chat will use its fallback." >&2
        return 0
    fi
    echo "==> Starting Ollama..."
    nohup "$ollama_bin" serve >>"$HOME/ollama/ollama-serve.log" 2>&1 &
    for i in $(seq 1 30); do
        if curl -s -o /dev/null -m 2 http://127.0.0.1:11434/api/tags 2>/dev/null; then
            echo "==> Ollama is up"
            return 0
        fi
        sleep 1
    done
    echo "run.sh: Ollama did not come up - see $HOME/ollama/ollama-serve.log" >&2
    return 0
}
ensure_ollama

# If the app is already running, just open the page.
if curl -s -o /dev/null http://127.0.0.1:3000/ 2>/dev/null; then
    echo "Already running - opening the page."
    print_links
    open_browser http://localhost:3000
    exit 0
fi

# Secrets live ONLY in .env (uncommitted, gitignored): this
# tracked script carries none. Load it now, then fail fast with
# a clear message when a required value is missing - the app
# also refuses to boot without them, but naming the missing
# key here saves a build cycle. On a fresh checkout, copy
# .env.example to .env and fill it in.
if [ ! -f .env ]; then
    echo "run.sh: .env is missing - copy .env.example to .env and fill in the values." >&2
    exit 1
fi
set -a
. ./.env
set +a
for key in DB_PASSWORD APP_DB_PASSWORD VERIFICATION_CODE_SECRET PII_ENCRYPTION_KEY; do
    if [ -z "${!key:-}" ]; then
        echo "run.sh: $key is not set in .env - see .env.example." >&2
        exit 1
    fi
done

if [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
fi


echo "==> Starting database container..."
docker compose up -d

echo "==> Waiting for the database..."
for i in $(seq 1 30); do
    if docker exec agentic-db pg_isready -U agentic -d agentic_commerce >/dev/null 2>&1; then
        break
    fi
    sleep 1
done

echo "==> Building and starting the server (first build takes a while)..."
cargo run -p server &
SERVER_PID=$!
trap "kill $SERVER_PID 2>/dev/null || true" INT TERM

for i in $(seq 1 90); do
    if curl -s -o /dev/null http://127.0.0.1:3000/ 2>/dev/null; then
        echo "==> Server is up"
        print_links
        open_browser http://localhost:3000
        break
    fi
    sleep 1
done

echo "    Press Ctrl+C to stop the server."
wait $SERVER_PID
