#!/bin/sh
set -eu
usage() { echo "usage: $0 NEW_TOOL_DIRECTORY COMMAND [ARG ...]" >&2; exit 2; }
[ "$#" -ge 2 ] || usage
tool_dir=$1
shift
command_name=$1
[ -n "$command_name" ] || usage
shift
command -v mise >/dev/null 2>&1 || { echo "missing dependency: mise" >&2; exit 127; }
user=$(id -un)
[ "$(id -u)" -ne 0 ] && [ -n "$user" ] && [ "$user" != root ] || { echo "run as a non-root user" >&2; exit 2; }
case "$(uname -s):$(uname -m)" in Linux:x86_64) arch=amd64; size=92221177; checksum=b9f18b8e37eb450e9e5df03117045237cdd12d3c4a0034ffeeaa857852a15e13 ;; \
  Linux:aarch64) arch=arm64; size=85048257; checksum=a4ad483a33b2f1544c5ccce1c5956c559e1a6e84b8bf6cedd847d695885237a3 ;; \
  *) echo "BuildKit bootstrap supports native Linux x86_64 and aarch64 only" >&2; exit 2 ;;
esac
for dep in python rootlesskit slirp4netns newuidmap newgidmap curl sha256sum tar; do
  command -v "$dep" >/dev/null 2>&1 || { echo "missing dependency: $dep" >&2; exit 127; }
done
case "$tool_dir" in ''|*/) { echo "tool directory must have a final path component" >&2; exit 2; } ;; esac
tool_parent=$(dirname -- "$tool_dir")
tool_name=$(basename -- "$tool_dir")
[ -d "$tool_parent" ] || { echo "tool directory parent must exist" >&2; exit 2; }
tool_parent=$(CDPATH='' cd -- "$tool_parent" && pwd -P)
tool_dir="$tool_parent/$tool_name"
[ ! -e "$tool_dir" ] && [ ! -L "$tool_dir" ] || { echo "tool directory already exists: $tool_dir" >&2; exit 2; }
mkdir -m 700 -- "$tool_dir"
created=1
mise_tmp=
cleanup() {
  result=$?
  trap - EXIT
  [ -z "$mise_tmp" ] || rm -rf -- "$mise_tmp"
  if [ "$result" -ne 0 ] && [ "$created" -eq 1 ]; then rm -rf -- "$tool_dir"; fi
  exit "$result"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
umask 077
mkdir "$tool_dir/mise-data" "$tool_dir/mise-cache" "$tool_dir/buildkit-bin"
mise_tmp=$(mktemp -d /tmp/ci-image-mise.XXXXXXXX)
mkdir "$mise_tmp/home" "$mise_tmp/config" "$mise_tmp/data" "$mise_tmp/cache"
: > "$mise_tmp/mise.toml"
tool="github:moby/buildkit[asset_pattern=buildkit-v0.34.2.linux-$arch.tar.gz,strip_components=0,bin_path=bin,checksum=sha256:$checksum,size=$size]@0.34.2"
run_mise() {
  env -i PATH="$PATH" USER="$user" HOME="$mise_tmp/home" XDG_CONFIG_HOME="$mise_tmp/config" \
    XDG_DATA_HOME="$mise_tmp/data" XDG_CACHE_HOME="$mise_tmp/cache" \
    MISE_CONFIG_FILE="$mise_tmp/mise.toml" MISE_DATA_DIR="$tool_dir/mise-data" MISE_CACHE_DIR="$tool_dir/mise-cache" \
    mise --cd "$mise_tmp" "$@"
}
run_mise install "$tool"
install_dir=$(run_mise where "$tool")
case "$install_dir" in /*) ;; *) echo "mise returned a non-absolute BuildKit install path" >&2; exit 1 ;; esac
[ -d "$install_dir/bin" ] && [ -x "$install_dir/bin/buildctl" ] && [ -x "$install_dir/bin/buildkitd" ] || {
  echo "installed BuildKit binaries are incomplete" >&2
  exit 1
}
for tool_path in "$install_dir"/bin/*; do
  [ -e "$tool_path" ] || continue
  case "${tool_path##*/}" in buildctl|buildkit*) ;; *) echo "unexpected binary in BuildKit install: ${tool_path##*/}" >&2; exit 1 ;; esac
done
launcher="$tool_dir/buildkit-bin/buildctl-daemonless.sh"
curl --fail --location --silent --show-error \
  --output "$launcher.tmp" \
  "https://raw.githubusercontent.com/moby/buildkit/8c579433824224dc64a158cf8734495b6e2b186a/examples/buildctl-daemonless/buildctl-daemonless.sh"
printf '%s  %s\n' 680e47bc3db72d10b092d41057575b60ec5fc0d6e784bb765cff8f6a7e3e5f6a "$launcher.tmp" | sha256sum --check --status
chmod 700 "$launcher.tmp"
mv -- "$launcher.tmp" "$launcher"
PATH="$tool_dir/buildkit-bin:$install_dir/bin:$PATH" "$command_name" "$@"
