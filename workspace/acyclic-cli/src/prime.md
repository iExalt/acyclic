# Working with the acyclic tracker (acyclic {version})

This block matches the installed `acyclic`; run `acyclic prime` again after an upgrade rather than relying on an
older copy.

## Who you are

- Set `ACYCLIC_ACTOR` to a name unique to this session, such as `session/<machine>-<session id>`, or `run/<id>`
  for a run. Two sessions sharing a name share claims, and both would work the same item. Without it you act as
  `agent/undeclared`.
- You are never the user. `--as-user` is for the user alone; don't pass it.

## Output

- Every command but `prime`, `--help` and `--version` prints one JSON document: `{"ok": true, "data": …}` or
  `{"ok": false, "error": {"code", "message", "details"}}`. Branch on `error.code`, not on the message.
- Exit codes: 0 done, 1 refused, 2 usage, 3 not found, 4 store (a lock, lost races or git; retry later),
  5 not set up, 6 unreadable state (run `acyclic doctor`).
- Writes report `changed`. A repeated command changes nothing of its own; it can still commit when a claim's lease
  is due for renewal.

## Finding and doing work

1. `acyclic ready` lists ready leaves, best first. `acyclic show <item>` explains one: its readiness, children and
   claims.
2. `acyclic claim <item>` before starting. A claim from anything but a run holds a time lease (60 minutes by
   default, `--lease <minutes>` up to 480). Your claim lapses after its lease unless you keep writing to the
   tracker; re-run `acyclic claim <item>` to renew it before then. Don't use `--force` to take someone else's work.
3. `acyclic done <item> --repository <record> --commit <id>` when finished, or `--note` when the result isn't a
   commit.
4. `acyclic add`, `edit`, `parent`, `link` and `unlink` shape the plan; `acyclic import <file>` applies a whole
   plan document, and `--dry-run` reports what it would change.

## Scope

- Edits to an approved thread's membership or repositories become proposals for the user, reported as
  `"outcome": "proposed"`. Carry on with other work; don't retry them.
- Approving threads, setting autonomy levels and deciding proposals are the user's. `acyclic needs` lists what is
  waiting on the user.
