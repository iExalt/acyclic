#!/bin/sh
# Invoke through scripts/with-ci-buildkit.sh with the project's mise-selected Python on PATH.
set -eu
usage() { echo "usage: $0 CONTEXT TARGET PLATFORM FINGERPRINT OUTPUT_DIRECTORY" >&2; exit 2; }
[ "$#" -eq 5 ] || usage
context=$1 target=$2 platform=$3 fingerprint=$4 output=$5
case "$target" in ''|*[!A-Za-z0-9_.-]*) usage ;; esac
case "$platform" in linux/amd64|linux/arm64) ;; *) echo "unsupported platform: $platform" >&2; exit 2 ;; esac
case "$fingerprint" in *[!0-9a-f]*|'') echo "fingerprint must be 64 lowercase hex characters" >&2; exit 2 ;; esac
[ "${#fingerprint}" -eq 64 ] || { echo "fingerprint must be 64 lowercase hex characters" >&2; exit 2; }
[ "$(id -u)" -ne 0 ] || { echo "run as a non-root user" >&2; exit 2; }
user=$(id -un)
[ -n "$user" ] && [ "$user" != root ] || { echo "a non-root user name is required" >&2; exit 2; }
case "$(uname -s):$(uname -m):$platform" in Linux:x86_64:linux/amd64|Linux:aarch64:linux/arm64) ;; *) echo "platform must match this native Linux host" >&2; exit 2 ;; esac
case "$output" in ''|*/) { echo "output directory must have a final path component" >&2; exit 2; } ;; esac
output_parent=$(dirname -- "$output")
output_name=$(basename -- "$output")
[ -d "$output_parent" ] || { echo "output parent must exist" >&2; exit 2; }
output_parent=$(CDPATH='' cd -- "$output_parent" && pwd -P)
output="$output_parent/$output_name"
[ ! -e "$output" ] && [ ! -L "$output" ] || { echo "output directory already exists: $output" >&2; exit 2; }
[ -d "$context" ] && [ -f "$context/Containerfile" ] || { echo "expected CONTEXT/Containerfile" >&2; exit 2; }
context=$(CDPATH='' cd -- "$context" && pwd -P)
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
for dep in rootlesskit slirp4netns buildctl buildkitd buildctl-daemonless.sh python; do
  command -v "$dep" >/dev/null 2>&1 || { echo "missing dependency: $dep" >&2; exit 127; }
done
case "$(mv --version 2>&1)" in *"GNU coreutils"*) ;; *) echo "GNU coreutils mv is required" >&2; exit 127 ;; esac
python_path=$(python -c 'import sys; print(sys.executable)')
[ -x "$python_path" ] || { echo "selected Python is not executable" >&2; exit 127; }
umask 077
tmp=$(mktemp -d "$output_parent/.ci-image.XXXXXXXX")
env_tmp=$(mktemp -d /tmp/ci-image-env.XXXXXXXX)
cleanup() { rm -rf -- "$tmp" "$env_tmp"; }
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
mkdir "$env_tmp/home" "$env_tmp/runtime" "$env_tmp/config" "$env_tmp/data" "$env_tmp/cache" "$env_tmp/docker"
set -- env -i PATH="$PATH" USER="$user" HOME="$env_tmp/home" \
  XDG_RUNTIME_DIR="$env_tmp/runtime" XDG_CONFIG_HOME="$env_tmp/config" \
  XDG_DATA_HOME="$env_tmp/data" XDG_CACHE_HOME="$env_tmp/cache" DOCKER_CONFIG="$env_tmp/docker" \
  ROOTLESSKIT='rootlesskit --net=slirp4netns --copy-up=/etc --disable-host-loopback' \
  BUILDKITD_FLAGS='--rootless --containerd-worker=false --oci-max-parallelism=4' \
  BUILDCTL_CONNECT_RETRIES_MAX=3
(
  cd -- "$tmp"
  "$@" buildctl-daemonless.sh build \
    --frontend dockerfile.v0 --local "context=$context" --local "dockerfile=$context" \
    --opt filename=Containerfile --opt "target=$target" --opt "platform=$platform" \
    --opt "label:io.acyclic.ci.fingerprint=$fingerprint" \
    --output type=oci,dest=image.oci.tar --metadata-file exporter.json
)
"$python_path" "$script_dir/validate-ci-image.py" "$tmp/image.oci.tar" "$tmp/exporter.json" "$target" "$platform" "$fingerprint" > "$tmp/manifest.json"
mkdir "$tmp/published"
mv -- "$tmp/image.oci.tar" "$tmp/exporter.json" "$tmp/manifest.json" "$tmp/published/"
mv -T -n -- "$tmp/published" "$output"
[ ! -e "$tmp/published" ] && [ ! -L "$tmp/published" ] || { echo "output appeared during build; refusing publication" >&2; exit 1; }
