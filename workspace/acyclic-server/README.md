# `acyclic-server`

The server behind `acyclic serve` and `acyclic up`: the tracker's JSON API and the app, for the user alone. Each
listener has its own check, and what passes one counts for nothing on another.

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
credential derived from the session, so rotating the session unpairs it too. Each server names its cookie for its
port, so pairing one doesn't unpair another. The cookie still reaches every port on localhost, so a command it
authenticates, a read as much as a write, must come from the listener's own origin, and no page may frame the app.

`up --tailscale --login <login> --host <name>.ts.net` also serves requests through `tailscale serve`, checked as `serve`
checks them: Tailscale replaces any `Tailscale-User-Login` a client sends with the user's, and passes the `.ts.net` name
on as `Host`, without a port. It listens on port 7479 of `127.0.0.1` (`--tailscale-port`), since Tailscale's Mac apps
run sandboxed and can't open a Unix socket. Every process and container on the machine can reach that port and send any
identity header, so the port answers only paths under a secret first segment, derived from the session, and takes it
off; anything else gets an empty 404. `up` prints the command that points Tailscale at it, `tailscale serve --bg
http://127.0.0.1:7479/<secret>`, and Tailscale prepends the secret to every path, so browsers never see it. Only what
can read the session can learn it, which keeps out containers as long as none mounts `~/.config/acyclic`; whatever keeps
`up`'s output, such as a service's log, must be the user's alone too. `--rotate` changes the secret, and the command
must be run again. With the open-source `tailscaled`, which can open one, `--socket <path>` listens on a Unix socket
only the user can open instead, with no secret, pointed at by `tailscale serve --bg unix:<path>`. The phone then reaches
the Mac while it is awake.

`up` doesn't sync on a timer unless given `--sync-every`, so the clone syncs when told to, as it does without a
server. It shares the clone with the CLI, whose writes queue with its own.

## The app

Both serve the app built into the binary (`acyclic-app-assets`), or the one in `--static <dir>`, which must hold an
`index.html`. `/` is `/index.html`, and a path with no file is 404, since the app has
no routes of its own. Files are sent Brotli- or gzip-compressed when the browser takes it and the build wrote the
copy. The bundle's hashed files under `/assets/` may be kept for good; everything else is revalidated. Every response
carries a content policy that allows only the server's own scripts, styles, workers and API, and no framing, except
the empty 404 that `up --tailscale` gives a path outside its secret.

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

## Notifications

The server tells the user's devices, through Web Push, when something new needs them: each entry of `acyclic needs`
whose `notify` is true, the first time it appears. `GET /api/push/key` gives the server's VAPID public key, `POST
/api/push/subscribe` takes a browser's `PushSubscription` as JSON (an `https://` endpoint and its `p256dh` and `auth`
keys), and `POST /api/push/unsubscribe` takes `{"endpoint": …}`. Only the user may call them, from the server's own
origin. Up to 10 devices are kept, the oldest replaced first.

After agents' writes, after syncs, and every minute besides, the server reads the list and, a few seconds later, sends
each device one encrypted message (RFC 8291) naming what is new; a push service keeps it for a day. What the user's own
writes through the app bring about is taken as seen. A device whose push service answers `404` or `410`, or refuses 5
sends in a row as malformed or unauthorised, is dropped. Being offline, or the service failing or deferring a send,
never drops a device: the news stays untold and is tried again with backoff, from 30 seconds up to 10 minutes.
The VAPID key, the devices and the needs already told live in `~/.config/acyclic/push`, readable only by the user, so a
restart tells nothing twice. Messages are signed (RFC 8292) with the server's origin behind Tailscale as the contact,
or what `~/.config/acyclic/push/subject` holds (`mailto:…` or `https://…`). TLS uses the system's certificate
authorities; without any, sends fail and the server says so once.
