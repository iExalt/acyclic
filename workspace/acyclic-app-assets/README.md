# `acyclic-app-assets`

The app's built files (`workspace/acyclic-app/dist`), embedded so the `acyclic` binary serves the app without a
directory beside it. `mise run build:app` writes them and nobody commits them. Without them the crate still builds, and
the server serves a page saying how to build the app; `ACYCLIC_REQUIRE_APP=1` (the app's checks and releases) demands
them instead. The crate is its own so that rebuilding the app recompiles only it, not the server.
