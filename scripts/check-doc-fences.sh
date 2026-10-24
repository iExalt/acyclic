#!/bin/sh
# Doctests don't run (nextest skips them), so every opening code fence in crate docs must name a non-Rust language
# from this list; rustdoc treats any other fence, tagged or not, as a doctest.
set -eu
allowed='text|sh|console|shell|json|toml|yaml|markdown|md|dot|mermaid|html|diff|csv'
status=0
for file in $(grep -rlE '```|~~~' workspace --include='*.rs' --include='README.md' --exclude-dir=template --exclude-dir=node_modules || true); do
  awk -v file="$file" -v allowed="^($allowed)$" '
    /^[[:space:]]*(\/\/[\/!][[:space:]]*)?(```|~~~)/ {
      info = $0
      sub(/^[[:space:]]*(\/\/[\/!][[:space:]]*)?(```+|~~~+)[[:space:]]*/, "", info)
      split(info, words, /[ ,]/)
      if (!open && words[1] !~ allowed) {
        printf "%s:%d: this code fence becomes a doctest that never runs; tag it with one of: %s\n", file, NR, allowed
        bad = 1
      }
      open = !open
    }
    END { exit bad }
  ' "$file" || status=1
done
exit $status
