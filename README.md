# acyclic

A decentralized, Git-native issue tracker and project management tool for one person and many agents.

Early days: nothing here works yet.

## North star

- Decentralized and Git-native: every machine keeps its own replica and works offline; replicas merge through Git.
- Workstreams and projects are represented as a DAG, with different types of edges: hard and soft dependencies,
  relationships and more.
- "Automatic" (well, model- or agent-assisted) placement and connection of new bugs, issues and feature requests as
  leaves in the DAG.
- [Paseo](https://github.com/getpaseo/paseo) integration: spawn one or more projects or worktrees to tackle an issue,
  a workstream, or a whole (sub)tree of issues.
- A CLI for agents and terminal interaction, and explicitly no MCP.
- A web UI for browsing, editing and adding issues.

The [glossary](GLOSSARY.md) defines the vocabulary.

## Licence

[MIT](LICENSE).
