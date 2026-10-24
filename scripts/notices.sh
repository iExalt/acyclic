#!/bin/sh
# Writes the release's notices to the path given: THIRD_PARTY.md, the licences of the Rust crates the `acyclic` binary
# links on every release target, and those of the packages the app's bundle holds, which `mise run build:app` lists.
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
out="$1"
crates=$(mktemp)
trap 'rm -f "$crates"' EXIT
cargo about generate --locked -c "$root/scripts/notices/about.toml" -m "$root/workspace/acyclic/Cargo.toml" \
  "$root/scripts/notices/crates.hbs" -o "$crates"
app="$root/workspace/acyclic-app/dist/licenses.txt"
[ -f "$app" ] || { echo "notices: $app is missing: run mise run build:app" >&2; exit 1; }
{ cat "$root/THIRD_PARTY.md"; echo; cat "$crates"; echo; echo '## The app'"'"'s packages'; echo; echo '```text'; cat "$app"; echo; echo '```'; } > "$out"
