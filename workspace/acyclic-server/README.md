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

## Runs, on `up --runners <port>`

`up --runners <port>` also listens on that port of `127.0.0.1` for runs, each presenting the token the operator gave
it as `Authorization: Bearer <token>`. It needs the operator's public key, `~/.config/acyclic/operator/key.pub`, which
the operator writes and only the user may be able to change, and the deny-list, `~/.config/acyclic/deny-list`; it
reads the key once, so a new operator key needs `up` restarted. `Host` isn't checked there: a container names the Mac
as `host.containers.internal`, and the check guards a browser's cookie, which no run has.

A run's token counts only there, and only while its run is under way. It reads the tracker as the run, and sends the
writes of its own work in its thread, each with its number in `Acyclic-Operation: <seq>`, counting from 1. A write is
scanned for the deny-list's terms and, with Gitleaks, for secrets, then applied as the run, once: sent again, it is
answered `duplicate`. It supersedes only what the tracker held when the run last read, as the server recorded it.
`GET /api/run` says the run, its thread and the latest of its operations applied.

## Runs' code

The runners' listener also serves git's smart HTTP protocol at `/git/<owner>/<name>.git`, so a run never holds a
credential for the forge. A run reaches only the repositories of its thread's set as it stands at each request; any
other is refused as not in this thread.

Each repository has a bare mirror in `$XDG_DATA_HOME/acyclic/mirrors` (by default under `~/.local/share`), made on
first use and fetched from `https://github.com/<owner>/<name>.git` (`ACYCLIC_UPSTREAM` names another forge) whenever a
run fetches, unless another fetch began in the last 5 seconds (`ACYCLIC_MIRROR_FRESH`), and every 5 minutes unless
`--sync-every 0`; when the forge can't be reached, runs get the mirror as last
fetched. Git runs there with no settings or credentials of the user's, so only public repositories can be mirrored for
now. Readiness checks whether a commit is merged against the mirror of a repository with no clone mapped in
`~/.config/acyclic/clones.toml`.

No run reaches a mirror itself: each thread has a repository of its own beside it, which borrows the mirror's objects
and holds its runs' pushes. A run sees the forge's branches and tags and its thread's runs' refs, so one replacing
another picks up its work, and nothing of another thread's. It pushes only to its own refs, `refs/runs/<run>/*`, and
only commits; a push's hook scans the new commits for the deny-list's terms, in a public repository, and with
Gitleaks, in every one. A request may take 10 minutes, a push 512 MiB and 64 refs, and a run one push at a time to
each repository.

Since the threads' repositories borrow its objects, a mirror keeps everything it fetched. Every 5 minutes, the
repositories of finished threads, those with every item done and no run under way, are removed once every run's ref
in them is on the forge, as the mirror has it, since until then they hold the only copy of that work; and a mirror no
thread borrows from any more drops what the forge no longer has.

The operator's `runs::environment` gives a run the settings that send every repository of the tracker's owners, by
any of the forge's URLs, to the server with the run's token.

## The app

Both serve the app built into the binary (`acyclic-app-assets`), or the one in `--static <dir>`, which must hold an
`index.html`; runs' listeners serve neither. `/` is `/index.html`, and a path with no file is 404, since the app has
no routes of its own. Files are sent Brotli- or gzip-compressed when the browser takes it and the build wrote the
copy. The bundle's hashed files under `/assets/` may be kept for good; everything else is revalidated. Every response
carries a content policy that allows only the server's own scripts, styles, workers and API, and no framing.

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
origin; never a run. Up to 10 devices are kept, the oldest replaced first.

After agents' writes, after syncs, and every minute besides, the server reads the list and, a few seconds later, sends
each device one encrypted message (RFC 8291) naming what is new; a push service keeps it for a day. What the user's own
writes through the app bring about is taken as seen. A device whose push service answers `404` or `410`, or refuses 5
sends in a row as malformed or unauthorised, is dropped. Being offline, or the service failing or deferring a send,
never drops a device: the news stays untold and is tried again with backoff, from 30 seconds up to 10 minutes.
The VAPID key, the devices and the needs already told live in `~/.config/acyclic/push`, readable only by the user, so a
restart tells nothing twice. Messages are signed (RFC 8292) with the server's origin behind Tailscale as the contact,
or what `~/.config/acyclic/push/subject` holds (`mailto:…` or `https://…`). TLS uses the system's certificate
authorities; without any, sends fail and the server says so once.
