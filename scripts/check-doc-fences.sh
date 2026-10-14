#!/bin/sh
# Doctests don't run (nextest skips them), so every opening code fence in crate docs must name a non-Rust language.
set -eu
status=0
for file in $(grep -rlE '```' workspace --include='*.rs' --include='README.md' --exclude-dir=template || true); do
  awk -v file="$file" '
    /^[[:space:]]*(\/\/[\/!][[:space:]]*)?```/ {
      info = $0
      sub(/^[[:space:]]*(\/\/[\/!][[:space:]]*)?```/, "", info)
      gsub(/[[:space:]]/, "", info)
      if (!open && (info == "" || info ~ /^rust/)) {
        printf "%s:%d: an untagged or Rust code fence becomes a doctest that never runs\n", file, NR
        bad = 1
      }
      open = !open
    }
    END { exit bad }
  ' "$file" || status=1
done
exit $status
