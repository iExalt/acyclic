#!/bin/sh
# Ask the pinned browser installer what is missing before changing the system. Only apt's requested packages use the
# dedicated archive cache; a cached .deb must never become an install request merely because it is in the directory.
set -eu
cache="${BROWSER_DEPS_CACHE:-$HOME/.cache/acyclic/browser-debs}/v2"
log=$(mktemp)
trap 'rm -f "$log"' EXIT INT TERM
start=$(date +%s)
result=0
uv run --no-project --with playwright==1.64.0 python -m playwright install-deps --dry-run chromium webkit >"$log" || result=$?
if [ "$result" -eq 0 ] && [ "$(cat "$log")" = 'All system dependencies are installed.' ]; then
  echo "browser deps: already installed in $(( $(date +%s) - start )) s"
  exit 0
fi
[ "$result" -eq 1 ] || { cat "$log" >&2; exit 1; }
# Parse data, never a shell command. Refuse changed installer output before invoking sudo.
packages=$(awk '
  NR == 1 && /^Missing system dependencies \([1-9][0-9]*\):$/ {
    expected = $0; sub(/^.*\(/, "", expected); sub(/\):$/, "", expected); next
  }
  NR > 1 && /^  [a-z0-9][a-z0-9+.-]*(:[a-z0-9][a-z0-9-]*)?$/ {
    sub(/^  /, ""); print; count++; next
  }
  { bad = 1 }
  END { if (bad || !expected || count != expected) exit 1 }
' "$log") || { cat "$log" >&2; exit 1; }
mkdir -p "$cache/partial"
sudo apt-get update
# Word splitting is intentional: every package name has passed the grammar above.
# shellcheck disable=SC2086
sudo apt-get -o "Dir::Cache::archives=$cache" install -y --no-install-recommends $packages
echo "browser deps: installed missing packages in $(( $(date +%s) - start )) s"
