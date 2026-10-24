#!/bin/sh
# Installs the app's dependencies from its lockfile: offline first, so a warm run never waits on the registry, and
# online only when pnpm's store lacks a package, saying so. Any other failure, such as a stale lockfile, fails here.
set -eu
cd "$(dirname "$0")/../workspace/acyclic-app"
log=$(mktemp)
trap 'rm -f "$log"' EXIT
if pnpm install --frozen-lockfile --offline --silent >"$log" 2>&1; then
  exit 0
fi
if grep -q 'ERR_PNPM_NO_OFFLINE' "$log"; then
  echo "app:install: pnpm's store lacks a package, so this run fetches online and isn't a warm one" >&2
  exec pnpm install --frozen-lockfile --silent
fi
cat "$log" >&2
exit 1
