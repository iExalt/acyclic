#!/bin/sh
# Pushes the server's per-architecture OCI archives, image-amd64.tar and image-arm64.tar in <dir>, to <repository> as
# <tag>-<arch>, then makes <tag> and `latest` one index of both, linked to this repository. Extra arguments go to
# skopeo, such as --dest-tls-verify=false for a local registry.
set -eu
repository="$1"
tag="$2"
dir="$3"
shift 3
for arch in amd64 arm64; do
  skopeo copy --all "$@" "oci-archive:$dir/image-$arch.tar" "docker://$repository:$tag-$arch"
done
docker buildx imagetools create \
  --annotation "index:org.opencontainers.image.source=https://github.com/iExalt/acyclic" \
  --tag "$repository:$tag" --tag "$repository:latest" \
  "$repository:$tag-amd64" "$repository:$tag-arm64"
