#!/bin/sh
set -eu
if [ "$#" -ne 2 ]; then
  echo 'usage: check-browser-artifact.sh SOURCE_ROOT ARTIFACT_ROOT' >&2
  exit 1
fi
tooling=$(CDPATH='' cd -- "$(dirname "$0")" && pwd)
source_root=$(CDPATH='' cd -- "$1" && pwd)
artifact_root=$(CDPATH='' cd -- "$2" && pwd)
cd "$source_root"
node "$tooling/frontend-artifact.mjs" install "$source_root" "$artifact_root"
sh "$tooling/check-browser.sh"
