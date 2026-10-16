# `acyclic-server`

The server behind `acyclic serve`: the tracker's JSON API and the app's static files, for the user alone.

Every request, whatever its path, must carry the user's identity in `Tailscale-User-Login`, which Tailscale sets from
its own lookup, and the server's name in `Host`; anything else is refused. The header proves nothing to a server that
can be reached without passing through Tailscale, so the server listens on localhost by default and warns when told
to listen anywhere else.

`POST /api/command` takes one of the CLI's commands as JSON, such as `{"show": {"items": ["a"]}}` or
`{"item": {"add": {"id": "a", "title": "A"}}}`, runs it as the user, and answers with the CLI's envelope. It runs every
read and write, but nothing that sets up or syncs the clone, or reads or writes the server's files. The tracker, the
actor and the clock are the server's. Requests must be `application/json`, and a browser's must come from the server's
own origin (`--origin`, `https://<host>` by default).

The server syncs its clone with `origin` every 30 seconds (`--sync-every`). Writes push at once only when the clone
says so, as for the CLI: set `git config acyclic.pushAfterWrite true` in a server's clone. Writes and syncs take turns,
so their pushes never race.
