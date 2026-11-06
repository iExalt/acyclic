#!/bin/sh
# Prints a short hash of the root Cargo.toml's [profile.*] sections, comments left out, for CI's Rust caches. Swatinem/rust-cache
# hashes the workspace members' manifests and Cargo.lock but not the root manifest, so a change of profile alone leaves its key
# as it was, and the next runs restore dependencies built under the old profile and rebuild all of them, without ever saving.
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
sed -n '/^\[profile\./,$p' "$root/Cargo.toml" | grep -v '^[[:space:]]*#' | shasum | cut -c1-12
