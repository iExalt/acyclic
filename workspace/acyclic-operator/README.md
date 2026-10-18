# `acyclic-operator`

The operator: what starts runs and stands between them and the tracker. A run never gets the tracker clone. It gets a
token naming it, signed by the operator's key in `~/.config/acyclic/operator/key`, and every write it sends is checked
against that token, so its writes are its own whatever it claims.

- `token` mints and checks run tokens.
- `forward` starts and ends runs and forwards their writes. A run sends operations: one of the CLI's commands each,
  numbered from 1, with its view of the tracker. The operator checks the token and that the run is still under way,
  forwards only the writes of a run's own work (claims, releases, done, item and link edits) within its thread, and the
  tracker applies each once, as the run, through the CLI's own checks. A command a run may not send is refused and its
  number spent.

A run's clone queues its writes rather than making them: with `git config acyclic.operations <run>`, each write a run
may send becomes `ops/<seq>.json`, holding `{"seq", "seen", "command"}`, added by one commit on the clone's `ops`
branch, and any other write is refused. `acyclic ops push` pushes the branch to `refs/runs/<run>/ops`. The receiver
reports each operation it forwarded on a line `acyclic-op <seq> ok`, `refused <code>` (spent) or `failed <code>` (not
spent), and ends with `acyclic-push forwarded <n>` or `acyclic-push refused <code>`; the clone marks how far the tracker
got. Push again after a failure that may pass; `acyclic ops reset` drops what the tracker didn't spend, to abandon it,
and waits while a push's outcome is unknown. `acyclic sync` in such a clone fetches and joins, and pushes nothing.
