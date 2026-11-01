# acyclic

A decentralized, Git-native issue tracker and project management tool for one person and many agents.

The CLI, the server and the app work. The north star below is wider than what is built: nothing starts runs yet, and
nothing runs agents in a cluster.

## North star

- Decentralized and Git-native: every machine keeps its own replica and works offline; replicas merge through Git.
- Threads and projects form a DAG with several kinds of edge: dependencies that hold work back, relationships and more.
- "Automatic" (well, model- or agent-assisted) placement and connection of new bugs, issues and feature requests as
  leaves in the DAG.
- Agent runs: spawn agents to tackle an issue, a thread, or a whole (sub)tree of issues, each run in its own pod in
  a Kubernetes cluster, with a dispatcher that starts ready work on its own. The design takes inspiration from
  [Paseo](https://github.com/getpaseo/paseo).
- A CLI for agents and terminal interaction, and explicitly no MCP.
- One app, installable on a phone, for browsing, editing and adding issues and for the agents working on them: an
  operations center that moves between the remaining work on the graph and the runs implementing it.

The tracker is a library with a CLI, and works without the app or the cluster.

The [glossary](GLOSSARY.md) defines the vocabulary.

A tracker syncs through a shared Git repository. On GitHub, `sh scripts/tracker-ruleset.sh <owner>/<repository>`
keeps that repository's audit logs from being force-pushed; run it before the first sync.

## Install

Each [release](https://github.com/iExalt/acyclic/releases) carries `acyclic` for Linux on x86-64 and arm64 (static)
and for macOS on Apple silicon. With [mise](https://mise.jdx.dev):

```sh
mise use -g github:iExalt/acyclic
```

mise holds a release back for its first day; name the version, such as `github:iExalt/acyclic@0.2.0`, to take it
sooner.

From a clone, `mise run install` fetches the graph page's renderers, builds the app and installs `acyclic` into
`~/.cargo/bin`.

The server's image is `ghcr.io/iexalt/acyclic-server`, tagged with each release's tag, such as `v0.2.0`, and `latest`.

## Licence

[MIT](LICENSE).
