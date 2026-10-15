#!/bin/sh
# Fetches the graph page's renderers from npm into the CLI's git-ignored assets, refusing any tarball or file whose
# SHA-256 differs from its pin in vendor-graph.pins. Files already matching their pins are kept, so this runs offline
# once fetched; new files land under a temporary name and are renamed into place, so a build never sees half a file.
# The renderers are never committed.
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
assets="$root/workspace/acyclic-cli/assets/graph"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

sha256() {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1"; else sha256sum "$1"; fi | cut -d' ' -f1
}

matches() {
  [ -f "$1" ] && [ "$(sha256 "$1")" = "$2" ]
}

place() {
  cp "$1" "$assets/.$2.$$"
  mv "$assets/.$2.$$" "$assets/$2"
}

mkdir -p "$assets"
grep -v '^#' "$root/scripts/vendor-graph.pins" | while read -r name url tarball file digest; do
  if matches "$assets/$name.js" "$digest" && [ -f "$assets/$name.LICENSE" ]; then
    continue
  fi
  mkdir -p "$work/$name"
  curl --fail --silent --show-error --location "$url" -o "$work/$name.tgz"
  matches "$work/$name.tgz" "$tarball" || {
    echo "vendor-graph: $name's tarball doesn't match its pin" >&2
    exit 1
  }
  tar -xzf "$work/$name.tgz" -C "$work/$name" "package/$file" package/LICENSE
  matches "$work/$name/package/$file" "$digest" || {
    echo "vendor-graph: $name's $file doesn't match its pin" >&2
    exit 1
  }
  place "$work/$name/package/$file" "$name.js"
  place "$work/$name/package/LICENSE" "$name.LICENSE"
done
