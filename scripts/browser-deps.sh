#!/bin/sh
# Installs the system libraries the browsers of check:browser need, which is what `playwright install-deps chromium webkit`
# installs: 181 packages and 114 MB on the CI image, from an apt mirror that took minutes for single packages on a bad day.
# With the packages of an earlier run in $BROWSER_DEPS_CACHE the install needs no network. Without them it downloads, as
# before, and then keeps the .deb files that apt left in its archive for the next run, which CI caches by Playwright version
# and runner image.
set -eu
cache="${BROWSER_DEPS_CACHE:-$HOME/.cache/acyclic/browser-debs}"
start=$(date +%s)
took() { echo "browser deps: $1 in $(( $(date +%s) - start )) s"; }

if ls "$cache"/*.deb >/dev/null 2>&1; then
  if sudo apt-get install -y --no-install-recommends --no-download "$cache"/*.deb; then
    took "installed $(ls "$cache"/*.deb | wc -l) cached packages"
    exit 0
  fi
  echo "browser deps: the cached packages don't install here; downloading instead" >&2
fi

uv run --no-project --with playwright==1.64.0 python -m playwright install-deps chromium webkit
mkdir -p "$cache"
cp /var/cache/apt/archives/*.deb "$cache"/ 2>/dev/null || echo "browser deps: apt kept no packages to cache" >&2
took "downloaded and installed, kept $(ls "$cache"/*.deb 2>/dev/null | wc -l) packages"
