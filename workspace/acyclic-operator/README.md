# `acyclic-operator`

The operator: what starts runs and stands between them and the tracker. A run never gets the tracker clone. It gets a
token naming it, signed by the operator's key in `~/.config/acyclic/operator/key`, and every write it sends is checked
against that token, so its writes are its own whatever it claims.

- `token` mints and checks run tokens.
- `forward` starts and ends runs and forwards their writes. A run sends operations: one of the CLI's commands each,
  numbered from 1, with its view of the tracker. The operator checks the token and that the run is still under way,
  forwards only the writes of a run's own work (claims, releases, done, item and link edits), and the tracker applies
  each once, as the run, through the CLI's own checks. A command a run may not send is refused and its number spent.
