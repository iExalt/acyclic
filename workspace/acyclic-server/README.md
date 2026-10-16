# `acyclic-server`

The server behind `acyclic serve`: the tracker's JSON API and the app's static files, for the user alone.

Every request, whatever its path, must carry the user's identity in `Tailscale-User-Login`, which Tailscale sets from
its own lookup, and the server's name in `Host`; anything else is refused. The header proves nothing to a server that
can be reached without passing through Tailscale, so the server listens on localhost by default and warns when told
to listen anywhere else.

`POST /api/command` takes one of the CLI's commands as JSON, such as `{"show": {"items": ["a"]}}`, and answers with
the CLI's envelope. Only the commands the server allows run; the tracker, the actor and the clock are the server's.
Requests must be `application/json`, and a browser's must come from the server's own origin.
