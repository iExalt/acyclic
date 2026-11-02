# Third-party code

The code this repository builds on but doesn't hold, and where its licences are listed.

## The app

The binary embeds the app, built by `mise run build:app` from `workspace/acyclic-app`, whose packages are pinned in its
`pnpm-lock.yaml`. Its bundle holds React and React DOM (MIT License), Cytoscape.js (MIT License) and elkjs (Eclipse
Public License 2.0) among others; the build lists every package it bundled, with its licence, in the app's
`licenses.txt`, which the server also serves.

## Release binaries

The Linux binaries are linked statically against musl, under the MIT License (<https://musl.libc.org>). Each release
archive carries `THIRD_PARTY_NOTICES.md`: this file followed by the licences of the Rust crates the binary links,
collected by `mise run notices` ([cargo-about](https://github.com/EmbarkStudios/cargo-about)), and those of the app's
bundled packages.

## The server's image

The image adds the binary and these notices to Alpine Linux, with its `git` and `ca-certificates` packages. Alpine's
packages keep their own licences: among them BusyBox and git are under the GNU General Public License version 2, and
musl under the MIT License. `/usr/share/acyclic/packages.txt` in the image lists every package and version it holds;
their sources are in Alpine's aports repository (<https://gitlab.alpinelinux.org/alpine/aports>) and on its mirrors.
