# `acyclic-server`

The server behind `acyclic serve` and `acyclic up`: the tracker's JSON API and the app's static files, for the user
alone. Each listener has its own check, and what passes one counts for nothing on another.

## `acyclic serve`, behind Tailscale

Every request, whatever its path, must carry the user's identity in `Tailscale-User-Login`, which Tailscale sets from
its own lookup, and the server's name in `Host`; anything else is refused. The header proves nothing to a server that
can be reached without passing through Tailscale, so the server listens on localhost by default and warns when told
to listen anywhere else. Even localhost is shared with every process and container on the machine, which can all
forge the header, so run `serve` only where Tailscale's proxy is the one thing that can reach it, such as a pod of its
own. On a Mac, use `acyclic up`.

## `acyclic up`, on the Mac

`up` listens on `127.0.0.1` only (`--port`, 7478 by default) and lets in requests presenting its session as
`Authorization: Bearer <session>`, with `Host` naming that port on `127.0.0.1`, `localhost` or `[::1]`. The session
lives in `~/.config/acyclic/up/session`, readable by the user alone, and lasts across restarts until
`acyclic up --rotate` replaces it. Never mount that directory into a container.

A browser pairs instead: `up` prints a link when it starts, and `acyclic up --link` (with the server's `--port`, if
it has one) prints a fresh one. A link is good once, for five minutes, and gives the browser a cookie with a
credential derived from the session, so rotating the session unpairs it too. The cookie reaches every port on
localhost, so a write it authenticates must come from the listener's own origin, and no page may frame the app.

`up --tailscale --login <login> --host <name>.ts.net` also listens on a Unix socket only the user can open
(`--socket`, `~/.config/acyclic/up/tailscale.sock` by default), checked as `serve` checks requests. Point Tailscale
at it with `tailscale serve --bg unix:<socket>`, as `up` prints; no container can reach the socket. The phone then
reaches the Mac while it is awake. Not yet checked against a real tailnet: that `tailscale serve` replaces any
`Tailscale-User-Login` a client sends and passes the original `Host` on, and that the Tailscale daemon can open a
socket under `~/.config`; if it can't, give `--socket` a path it can reach.

`up` doesn't sync on a timer unless given `--sync-every`, so the clone syncs when told to, as it does without a
server. It shares the clone with the CLI, whose writes queue with its own.

## The API

`POST /api/command` takes one of the CLI's commands as JSON, such as `{"show": {"items": ["a"]}}` or
`{"item": {"add": {"item": "a", "title": "A"}}}`, runs it as the user, and answers with the CLI's envelope. It runs
every read and write, but nothing that sets up or syncs the clone, or reads or writes the server's files. The tracker,
the actor and the clock are the server's. Requests must be `application/json`, and a browser's must come from the
server's own origin (`--origin`, `https://<host>` by default, behind Tailscale).

The server syncs its clone with `origin` every 30 seconds (`--sync-every`; never, for `up`). Writes push at once only
when the clone says so, as for the CLI: set `git config acyclic.pushAfterWrite true` in a server's clone. Writes and
syncs take turns, so their pushes never race; a write that waits 15 seconds for its turn is answered `503` with the
code `busy`, and git gives up on a transfer that stalls for 30 seconds. A clone without `origin` never syncs; one that
loses its `origin` stops syncing until the server restarts.
