#!/bin/sh
# Writes the release's notices to the path given: THIRD_PARTY.md, then the licences of the Rust crates the `acyclic`
# binary links on every release target.
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
out="$1"
crates=$(mktemp)
trap 'rm -f "$crates"' EXIT
cargo about generate --locked -c "$root/scripts/notices/about.toml" -m "$root/workspace/acyclic/Cargo.toml" \
  "$root/scripts/notices/crates.hbs" -o "$crates"
{ cat "$root/THIRD_PARTY.md"; echo; cat "$crates"; } > "$out"
