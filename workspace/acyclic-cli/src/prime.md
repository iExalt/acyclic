# Working with the acyclic tracker (acyclic {version})

This block matches the installed `acyclic`; run `acyclic prime` again after an upgrade rather than relying on an
older copy.

## Who you are

- Set `ACYCLIC_ACTOR` to a name unique to this session, such as `session/<machine>-<session id>`, or `run/<id>`
  for a run. Two sessions sharing a name share claims, and both would work the same item. Without it you act as
  `agent/undeclared`.
- You are never the user. `--as-user` is for the user alone; don't pass it.

## Output

- Times are RFC 3339 in UTC, such as `2026-10-15T09:30:00Z`.
- Every command but `prime`, `--help`, `--version` and a bare group (which prints its help) prints one JSON
  document: `{"ok": true, "data": …}` or `{"ok": false, "error": {"code", "message", "details"}}`. Branch on
  `error.code`, not on the message.
- A command line that doesn't parse prints its error as text on standard error, with exit code 2; run
  `acyclic <command> --help` to see the command's arguments.
- Exit codes: 0 done, 1 refused, 2 usage, 3 not found, 4 store (a lock, lost races or git; retry later),
  5 not set up, 6 unreadable state (run `acyclic doctor`).
- Writes report `changed`. A repeated command changes nothing of its own; it can still commit when a claim's lease
  is due for renewal.

## Finding and doing work

1. `acyclic ready` lists ready leaves, best first, each with its thread and whether the user approved its work;
   `acyclic ready --approved` lists only approved work. `acyclic list` lists every item. `acyclic show <item>...`
   explains items, as an array in the order given: their readiness, children and claims.
2. `acyclic claim <item>` before starting. A claim from anything but a run holds a time lease (60 minutes by
   default, `--lease <minutes>` up to 480). Your claim lapses after its lease unless you keep writing to the
   tracker; re-run `acyclic claim <item>` to renew it before then. Don't use `--force` to take someone else's work.
   If `claim`, `ready` or `show` says another holder wins a contest for your item, stop and run
   `acyclic release <item>`: it drops only your claim.
3. `acyclic done <item> --repository <record> --commit <id>` when finished, or `--note` when the result isn't a
   commit. It releases your claim. It finishes only what you hold, or what is ready, and a parent once its children
   are done; don't use `--force` to finish someone else's work.
4. `acyclic item` (`add`, `edit`, `move`, `state`) and `acyclic link` (`add`, `remove`) shape the plan;
   `acyclic import <file>` applies a whole plan document, and `--dry-run` reports what it would change.
   `acyclic item add` returns `candidates`: likely parents, threads and archived items to relate to, best first,
   each with the command that accepts it; with `--parent`, `parent_rank` says where your choice ranks.
   `acyclic graph --output <file>` writes the graph as DOT or Mermaid; on a large tracker, read the file rather than
   standard output.
5. `acyclic thread` and `acyclic repo` group work into threads and attach repositories. `acyclic <group> --help`
   lists a group's commands.
6. Grouping work into threads is yours: create threads with `acyclic thread set`, fill them with `acyclic thread
   add`, give each a title and brief, and leave approving them to the user. New or moved work joins the thread
   its parent passes down, or else the one its siblings share; an add, move or import reports it as `inherited`.
7. `acyclic sync` exchanges work with the tracker's shared repository (`origin`): it fetches every replica's state,
   joins it, and pushes yours; `changed` says whether joining changed this clone and `pushed` counts the refs sent. Run
   it before reading what another machine wrote and after writing what one should see. With
   `git config acyclic.pushAfterWrite true` in the tracker clone, every write pushes too and reports `push`; a push that
   fails leaves the write in place and shows in `acyclic needs` until a sync.
