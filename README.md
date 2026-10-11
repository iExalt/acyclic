# acyclic

A decentralized, Git-native issue tracker and project management tool for one person and many agents.

Early days: nothing here works yet.

## North star

- Decentralized and Git-native: every machine keeps its own replica and works offline; replicas merge through Git.
- Threads and projects are represented as a DAG, with different types of edges: hard and soft dependencies,
  relationships and more.
- "Automatic" (well, model- or agent-assisted) placement and connection of new bugs, issues and feature requests as
  leaves in the DAG.
- Agent runs: spawn agents to tackle an issue, a thread, or a whole (sub)tree of issues, each run in its own pod in
  a Kubernetes cluster, with a dispatcher that starts ready work on its own. The design takes inspiration from
  [Paseo](https://github.com/getpaseo/paseo).
- A CLI for agents and terminal interaction, and explicitly no MCP.
- One app, installable on a phone, for browsing, editing and adding issues and for the agents working on them: an
  operations center that moves between the remaining work on the graph and the runs implementing it.

The tracker itself is a library with its CLI, and works without the app or the cluster.

The [glossary](GLOSSARY.md) defines the vocabulary.

## Licence

[MIT](LICENSE).
