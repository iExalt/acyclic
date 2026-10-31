#!/bin/sh
# Serves the app from Vite's dev server, reloading as its source changes, against `acyclic up` on a fixture tracker in a
# temporary home: Vite passes the API and the pairing link through to the server. Prints the link to pair a browser
# with, then runs until interrupted, removing the temporary home on the way out.
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
export HOME="$home" ACYCLIC_TRACKER="$home/tracker" ACYCLIC_ACTOR=session/app-dev ACYCLIC_OUTPUT=json
"$root/target/debug/acyclic-fixtures" --items "$items" --seed "$seed" >"$home/plan.json"
"$acyclic" init "$ACYCLIC_TRACKER" --replica dev >/dev/null
"$acyclic" --as-user import "$home/plan.json" >/dev/null
fifo="$home/up"
mkfifo "$fifo"
"$acyclic" up --port 0 >"$fifo" &
server=$!
read -r line <"$fifo"
link=$(printf '%s' "$line" | sed -n 's/.*"link":"\([^"]*\)".*/\1/p')
origin=$(printf '%s' "$link" | sed 's#^\(https\{0,1\}://[^/]*\).*#\1#')
port=${ACYCLIC_DEV_PORT:-5173}
echo "Pair at: http://localhost:$port${link#"$origin"}"
cd "$root/workspace/acyclic-app"
ACYCLIC_UP="$origin" pnpm exec vite --port "$port" --strictPort
