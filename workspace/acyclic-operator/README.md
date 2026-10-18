# `acyclic-operator`

The operator: what starts runs and stands between them and the tracker. A run never gets the tracker clone. It gets a
token naming it and its thread, signed by the operator's key in `~/.config/acyclic/operator/key`, and every request it
makes to the server presents that token, so its writes are its own whatever it claims.

- `token` mints and checks run tokens.
- `runs` starts and ends runs. Starting one records it in the tracker and mints its token; ending one revokes the token,
  since the server refuses a run that isn't under way.
