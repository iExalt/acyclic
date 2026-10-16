# Third-party code

`acyclic graph --format html` (and `--open`) writes a page that embeds one of two renderers. Neither is in this
repository: `mise run vendor:graph` (`scripts/vendor-graph.sh`) fetches them, pinned in `scripts/vendor-graph.pins`,
from npm into `workspace/acyclic-cli/assets/graph/`, refusing any file whose SHA-256 differs from its pin below, and the
build embeds them in the binary. Each page carries a notice naming the renderer's licences and sources.

## Graphviz, through @hpcc-js/wasm-graphviz

- Package: @hpcc-js/wasm-graphviz 1.29.2, embedding Graphviz 16.1.0 compiled to WebAssembly.
- Licences: the wrapper is under the Apache License 2.0; Graphviz is under the Eclipse Public License 1.0.
- Sources: <https://github.com/hpcc-systems/hpcc-js-wasm> and <https://gitlab.com/graphviz/graphviz>.
- Tarball: <https://registry.npmjs.org/@hpcc-js/wasm-graphviz/-/wasm-graphviz-1.29.2.tgz>, SHA-256
  `fbabf5f7a085f37009477d3585b2da1e4960f99dda54004e69c20553c8d172df`.
- File: `package/dist/index.js`, SHA-256 `550d9cd6ff429f5bd225316fe1faed4f852038c3e1ec609ce113af1f9379ce25`.

## Mermaid

- Package: mermaid 12.2.0, its single-file browser bundle.
- Licences: Mermaid is under the MIT License. The bundle includes, among others, elkjs (Eclipse Public License 2.0),
  DOMPurify (Apache License 2.0 or Mozilla Public License 2.0), d3 (ISC License), dagre-d3-es (MIT License), cytoscape
  (MIT License) and KaTeX (MIT License); the package's `package.json` lists them all.
- Sources: <https://github.com/mermaid-js/mermaid> and <https://github.com/kieler/elkjs>.
- Tarball: <https://registry.npmjs.org/mermaid/-/mermaid-12.2.0.tgz>, SHA-256
  `071914164d0f6d53e5c26a10a220230d4ae8d37ed0f2263fbf072266f0af3a0a`.
- File: `package/dist/mermaid.min.js`, SHA-256 `a8a6a6f805b074a8261a2825c86f99c8054acb3b35f50bb7e24a4f9f2c3c3b9f`.

## Release binaries

The Linux binaries are linked statically against musl, under the MIT License (<https://musl.libc.org>). Each release
archive carries `THIRD_PARTY_NOTICES.md`: this file followed by the licences of the Rust crates the binary links,
collected by `mise run notices` ([cargo-about](https://github.com/EmbarkStudios/cargo-about)).

## The server's image

The image adds the binary and these notices to Alpine Linux, with its `git` and `ca-certificates` packages. Alpine's
packages keep their own licences: among them BusyBox and git are under the GNU General Public License version 2, and
musl under the MIT License. `/usr/share/acyclic/packages.txt` in the image lists every package and version it holds;
their sources are in Alpine's aports repository (<https://gitlab.alpinelinux.org/alpine/aports>) and on its mirrors.
