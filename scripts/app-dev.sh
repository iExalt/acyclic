#!/bin/sh
# Serves the app from Vite's dev server, reloading as its source changes, against `acyclic up` on a fixture tracker in a
# temporary home: Vite passes the API through to the server. Prints the app's address, then runs until interrupted,
# removing the temporary home on the way out.
#
# Usage: app-dev.sh [items] [seed]
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
items=${1:-350}
seed=${2:-7}
cargo build --quiet --locked -p acyclic -p acyclic-fixtures
acyclic="$root/target/debug/acyclic"
home=$(mktemp -d)
server=""
trap '[ -z "$server" ] || kill "$server" 2>/dev/null; rm -rf "$home"' EXIT INT TERM
export HOME="$home" ACYCLIC_TRACKER="$home/tracker" ACYCLIC_ACTOR=session/app-dev
"$root/target/debug/acyclic-fixtures" --items "$items" --seed "$seed" >"$home/plan.json"
"$acyclic" init "$ACYCLIC_TRACKER" --replica dev >/dev/null
"$acyclic" import "$home/plan.json" >/dev/null
fifo="$home/up"
mkfifo "$fifo"
"$acyclic" up --port 0 >"$fifo" &
server=$!
read -r line <"$fifo"
origin=http://$(printf '%s' "$line" | sed -n 's/.*"listening":"\([^"]*\)".*/\1/p')
port=${ACYCLIC_DEV_PORT:-5173}
echo "Open: http://localhost:$port/"
cd "$root/workspace/acyclic-app"
ACYCLIC_UP="$origin" pnpm exec vite --port "$port" --strictPort
