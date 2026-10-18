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
- `endpoint` is where runs push: git's smart HTTP protocol at `/tracker.git`, `git-receive-pack` alone, with the run's
  token as `Authorization: Bearer <token>` on every request. Each run pushes to an inbox of its own under
  `~/.config/acyclic/operator/inbox/`, which refuses deletions and rewrites and runs only the operator's hook.
- `hook` is that hook, run as `acyclic operator-hook`. Before forwarding anything it checks the whole push: one update
  to the run's own ref, a chain of commits that each add the next operation and nothing else, and nothing the
  deny-list in `~/.config/acyclic/deny-list` or Gitleaks objects to. Gitleaks must be on the operator's `PATH`, as
  itself rather than a shim that needs a project to resolve; without it every push is refused.

Nothing serves the endpoint yet, and `acyclic up` checks no run tokens: runs reach the tracker through the operator
alone.

A run's clone queues its writes rather than making them: with `git config acyclic.operations <run>`, each write a run
may send becomes `ops/<seq>.json`, holding `{"seq", "seen", "command"}`, added by one commit on the clone's `ops`
branch, and any other write is refused. `acyclic ops push` pushes the branch to `refs/runs/<run>/ops`, at most 200
operations past the last push taken at a time. The receiver reports each operation it forwarded on a line
`acyclic-op <seq> ok`, `refused <code>` (spent) or `failed <code>` (not spent), and ends with
`acyclic-push forwarded <n>` or `acyclic-push refused <code>`; the clone marks how far the tracker got. Push again after
a failure that may pass, the operator's own included; `acyclic ops reset` drops what the tracker didn't spend, to
abandon what the operator will never take, and waits while a push's outcome is unknown. `acyclic sync` in such a clone
fetches and joins, and pushes nothing.
