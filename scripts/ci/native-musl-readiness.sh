#!/bin/sh
# Checks that a native Linux runner can compile, statically link and run a Rust+C musl fixture.
set -eu

fail() {
  printf 'native musl readiness: %s\n' "$*" >&2
  exit 1
}

[ "$#" -eq 2 ] || fail 'usage: native-musl-readiness.sh TARGET OUTPUT_DIR'
TARGET=$1
OUTPUT_DIR=$2

case "$TARGET" in
  x86_64-unknown-linux-musl)
    expected_arch=x86_64
    expected_machine='Advanced Micro Devices X86-64'
    cc_name=CC_x86_64_unknown_linux_musl
    selected_cc=${CC_x86_64_unknown_linux_musl:-}
    ;;
  aarch64-unknown-linux-musl)
    expected_arch=aarch64
    expected_machine=AArch64
    cc_name=CC_aarch64_unknown_linux_musl
    selected_cc=${CC_aarch64_unknown_linux_musl:-}
    ;;
  *)
    fail "unsupported target: $TARGET"
    ;;
esac

[ "$(id -u)" -ne 0 ] || fail 'must not run as root'
[ -n "${HOME:-}" ] || fail 'HOME is missing'
[ -d "$HOME" ] && [ -w "$HOME" ] || fail 'HOME must be an existing writable directory'
[ -n "$OUTPUT_DIR" ] || fail 'OUTPUT_DIR is missing'
[ -d "$OUTPUT_DIR" ] && [ -w "$OUTPUT_DIR" ] || fail 'OUTPUT_DIR must be an existing writable directory'
[ -n "${EXPECTED_RUST_VERSION:-}" ] || fail 'EXPECTED_RUST_VERSION is missing'
[ -n "${EXPECTED_CARGO_ABOUT_VERSION:-}" ] || fail 'EXPECTED_CARGO_ABOUT_VERSION is missing'

for tool in rustc cargo rustup ar readelf file tar gzip jq awk grep sort mktemp cmp id mkdir rm cat uname skopeo; do
  command -v "$tool" >/dev/null 2>&1 || fail "required tool is missing: $tool"
done
[ "$selected_cc" = musl-gcc ] || fail "$cc_name must be musl-gcc"
command -v "$selected_cc" >/dev/null 2>&1 || fail 'musl-gcc is missing'
skopeo --version >/dev/null 2>&1 || fail 'skopeo --version failed'

runner_arch=$(uname -m)
[ "$runner_arch" = "$expected_arch" ] || fail "runner architecture $runner_arch does not match $TARGET"
rustc_host=$(rustc -vV | awk '$1 == "host:" { print $2; exit }')
case "$rustc_host" in
  "$expected_arch"-*-linux-*) ;;
  *) fail "Rust host $rustc_host does not match native target $TARGET" ;;
esac
rustc_version=$(rustc --version | awk '{ print $2 }')
[ "$rustc_version" = "$EXPECTED_RUST_VERSION" ] || fail "Rust version $rustc_version does not match expected version $EXPECTED_RUST_VERSION"
rustup target list --installed | grep -F -x "$TARGET" >/dev/null || fail "Rust target is not installed: $TARGET"
cc_machine=$("$selected_cc" -dumpmachine) || fail 'musl-gcc could not report its target architecture'
case "$cc_machine" in
  "$expected_arch"-*) ;;
  *) fail "musl-gcc architecture $cc_machine does not match $TARGET" ;;
esac
about_version=$(cargo about --version | awk 'NF { print $NF; exit }')
[ "$about_version" = "$EXPECTED_CARGO_ABOUT_VERSION" ] || fail "cargo-about version $about_version does not match expected version $EXPECTED_CARGO_ABOUT_VERSION"

case "$OUTPUT_DIR" in
  /) fixture_parent=/native-musl-readiness.XXXXXX ;;
  *) fixture_parent=${OUTPUT_DIR%/}/native-musl-readiness.XXXXXX ;;
esac
fixture_dir=$(mktemp -d "$fixture_parent") || fail 'could not create fixture directory under OUTPUT_DIR'
trap 'rm -rf "$fixture_dir"' 0
trap 'exit 1' HUP INT TERM

rustup component list --installed | LC_ALL=C sort > "$fixture_dir/rust-components"
printf '%s\n' "cargo-$rustc_host" "rustc-$rustc_host" "rust-std-$rustc_host" "rust-std-$TARGET" | LC_ALL=C sort > "$fixture_dir/expected-components"
cmp "$fixture_dir/expected-components" "$fixture_dir/rust-components" || fail 'installed Rust components differ from the minimal native set'

metadata_dir=$fixture_dir/metadata
dependency_dir=$fixture_dir/dependency
mkdir -p "$metadata_dir/src" "$dependency_dir/src"
cat > "$metadata_dir/Cargo.toml" <<'TOML'
[package]
name = "native-readiness-fixture"
version = "0.0.0"
edition = "2021"
license = "MIT"

[dependencies]
native-readiness-license = { path = "../dependency" }

[workspace]
TOML
printf '%s\n' 'fn main() {}' > "$metadata_dir/src/main.rs"
cat > "$dependency_dir/Cargo.toml" <<'TOML'
[package]
name = "native-readiness-license"
version = "0.1.0"
edition = "2021"
license = "MIT"
TOML
printf '%s\n' 'pub fn license_probe() {}' > "$dependency_dir/src/lib.rs"
printf '%s\n' 'MIT License' 'Synthetic readiness fixture.' > "$dependency_dir/LICENSE"
metadata=$(cargo metadata --offline --format-version 1 --manifest-path "$metadata_dir/Cargo.toml") || fail 'synthetic Cargo metadata failed'
printf '%s\n' "$metadata" | jq -e '.packages | map(select(.license == "MIT")) | length == 2' >/dev/null || fail 'synthetic MIT package metadata did not match'

cat > "$fixture_dir/about.toml" <<TOML
accepted = ["MIT"]
targets = ["$TARGET"]
ignore-build-dependencies = true
ignore-dev-dependencies = true
TOML
cat > "$fixture_dir/notices.hbs" <<'HBS'
{{#each licenses}}
License: {{name}}
{{#each used_by}}
Package: {{crate.name}} {{crate.version}}
{{/each}}
{{{text}}}
{{/each}}
HBS
cargo about generate --offline -m "$metadata_dir/Cargo.toml" -c "$fixture_dir/about.toml" "$fixture_dir/notices.hbs" -o "$fixture_dir/generated-notices.md" || fail 'offline synthetic license generation failed'
grep -F 'License: MIT License' "$fixture_dir/generated-notices.md" >/dev/null || fail 'generated notices omitted the MIT license identity'
grep -F 'native-readiness-license 0.1.0' "$fixture_dir/generated-notices.md" >/dev/null || fail 'generated notices omitted the synthetic dependency'

package_dir=$fixture_dir/package
mkdir "$package_dir"
printf '%s\n' 'MIT License' 'Synthetic readiness fixture.' > "$package_dir/LICENSE"
cat "$fixture_dir/generated-notices.md" > "$package_dir/THIRD_PARTY_NOTICES.md"
archive=$fixture_dir/notices.tar.gz
tar -czf "$archive" -C "$package_dir" LICENSE THIRD_PARTY_NOTICES.md || fail 'tar/gzip packaging failed'
gzip -t "$archive" || fail 'gzip archive validation failed'
tar -tzf "$archive" > "$fixture_dir/archive-list"
printf '%s\n' LICENSE THIRD_PARTY_NOTICES.md > "$fixture_dir/expected-list"
cmp "$fixture_dir/expected-list" "$fixture_dir/archive-list" || fail 'archive contents did not match'
mkdir "$fixture_dir/extracted"
tar -xzf "$archive" -C "$fixture_dir/extracted" || fail 'tar/gzip extraction failed'
cmp "$package_dir/LICENSE" "$fixture_dir/extracted/LICENSE" || fail 'LICENSE round-trip failed'
cmp "$package_dir/THIRD_PARTY_NOTICES.md" "$fixture_dir/extracted/THIRD_PARTY_NOTICES.md" || fail 'notices round-trip failed'

cat > "$fixture_dir/probe.c" <<'C'
#include <string.h>

#ifdef __GLIBC__
#error "musl headers are required"
#endif

size_t native_readiness_probe(const char *value) {
  return strlen(value);
}
C
"$selected_cc" -fno-builtin -c "$fixture_dir/probe.c" -o "$fixture_dir/probe.o" || fail 'musl C compilation failed'
ar crs "$fixture_dir/libprobe.a" "$fixture_dir/probe.o" || fail 'C archive creation failed'
cat > "$fixture_dir/main.rs" <<'RS'
#[link(name = "probe", kind = "static")]
unsafe extern "C" {
    fn native_readiness_probe(value: *const std::ffi::c_char) -> usize;
}

fn main() {
    let input = b"musl\0";
    let length = unsafe { native_readiness_probe(input.as_ptr() as *const std::ffi::c_char) };
    if length != 4 {
        std::process::exit(1);
    }
}
RS
rustc --edition=2021 --target "$TARGET" -C linker="$selected_cc" -L "native=$fixture_dir" -l static=probe "$fixture_dir/main.rs" -o "$fixture_dir/native-readiness" || fail 'Rust+C musl static link failed'

readelf -h "$fixture_dir/native-readiness" > "$fixture_dir/elf-header" || fail 'readelf could not inspect the fixture'
elf_machine=$(awk -F: '/Machine:/ { sub(/^[^:]*:[[:space:]]*/, ""); print; exit }' "$fixture_dir/elf-header")
[ "$elf_machine" = "$expected_machine" ] || fail "fixture ELF machine $elf_machine did not match $expected_machine"
readelf -l "$fixture_dir/native-readiness" > "$fixture_dir/program-headers" || fail 'readelf could not inspect program headers'
if grep -F 'INTERP' "$fixture_dir/program-headers" >/dev/null; then
  fail 'fixture has a dynamic interpreter'
fi
file "$fixture_dir/native-readiness" | grep -E 'statically linked|static-pie linked' >/dev/null || fail 'fixture is not reported as statically linked'
"$fixture_dir/native-readiness" || fail 'native fixture execution failed'
printf 'native musl readiness passed: %s\n' "$TARGET"
