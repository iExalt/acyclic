# `acyclic-operator`

The operator: what starts runs and stands between them and the tracker. A run never gets the tracker clone. It gets a
token naming it, signed by the operator's key in `~/.config/acyclic/operator/key`, and every write it sends is checked
against that token, so its writes are its own whatever it claims.

So far it mints and checks run tokens (`token`).
