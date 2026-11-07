# Working with the acyclic tracker (acyclic {version})

This block matches the installed `acyclic`; run `acyclic prime` again after an upgrade rather than relying on an
older copy.

## The loop

Set `ACYCLIC_ACTOR` first (see "Who you are"), then:

```sh
acyclic thread ready
acyclic thread claim <thread>
# ...work its ready items in order, finishing each:
acyclic item done <item> --commit <owner/name>@<sha>
acyclic item done <item> --evidence 'what was done'
# ...and when none of its items is left ready:
acyclic thread release <thread>
```

A thread is the unit of work: you claim a whole thread, and you and your subagents work every member without
claiming items. `thread ready` lists threads with ready work, best first, each with its ready `items` in order. Finish
with `--commit`, naming the repository that holds the commit and the commit, or with `--evidence` when the result isn't
a commit.

## Naming records

Every item, thread and repository record has a UUIDv7 id. Wherever a command takes one record, name it by its full id,
by its exact title (an item's or a thread's) or `owner/name` (a repository's, in any case), or by the last 6 or more
characters of its id, as text output prints it. A reference that names several records, or none, is refused, and the
error's details list the candidates. Put a title in single quotes, as `'Plant the seeds'`, so the shell leaves its
quotes and backticks alone, or use an id ending.

## Setting up a clone

`acyclic init --replica <id> <path> --remote <url>` creates a tracker clone at `<path>` as a new replica, with the
shared repository as its `origin`. Choose an id no other clone has used. `acyclic sync` then pulls in what other
machines recorded and pushes yours.

## Who you are

- Set `ACYCLIC_ACTOR` to a name unique to this session, such as `session/<machine>-<session id>`. Two sessions
  sharing a name share claims, and both would work the same thread. Without it, or `--actor`, your writes are
  labelled as the user's, `actor/user`.

## Waits for

`acyclic item edit 'Plant' --waits-for 'Beds'` means Plant waits for Beds: it can't start until Beds is done.
`--no-waits-for 'Beds'` undoes it, and `acyclic item add` takes `--waits-for` too. An edit that would make items wait
for each other in a circle is refused.

Only waits-for orders work: ids, titles and thread membership never do, so say every dependency you mean.

## Shaping the plan

`acyclic item add` and `acyclic item edit` take the same field flags:

- `--field <VALUE>` sets a scalar field, or adds to a set; repeat it for more.
- `--no-field` clears a scalar field and takes no value. `--no-field <VALUE>` removes from a set; repeat it for more.
- Naming `--field` and `--no-field` together is a usage error.

For example, `acyclic item edit 'Beds' --thread 'Garden' --no-notes --waits-for 'Soil' --no-waits-for 'Clay'` moves
Beds into the Garden thread, clears its notes, makes it wait for Soil and no longer for Clay. Thread membership is
`acyclic item edit --thread` (`--no-thread` takes the item out of its thread). `--state` sets `open`, `parked` or
`canceled`: canceled drops the item, which then counts as neither done nor open and puts work waiting on it on `needs`;
done work is reopened before it can be canceled, `--force` cancels over another holder's claim, and `--state open`
undoes it. `item done` sets `done` on a leaf, and `item reopen` makes a done leaf open again; a parent is done once
its leaves are, and is never finished or reopened by hand. What is under way shows in an item's
`stage`, from its thread's claim, not in its state.

Leave out `--id` in `acyclic item add`, `acyclic thread add` and `acyclic repository add`: the tracker makes a UUIDv7
and prints it. To know an id before adding, as a plan written ahead does, take new ones from `acyclic id [N]`, which
prints N of them and needs no tracker, and pass each as `--id`; any other id is refused. If an add fails
`already-exists`, a thread (for `thread add`) or an unfinished item in the same initiative (for `item add`) has that
title, most likely from an earlier try; the error's details give its id. Titles are how people find work, so two
unfinished items in one initiative never share one: adding, renaming, moving, reopening and un-canceling all refuse a
duplicate. Finishing a leaf frees its title, but a parent keeps its name even when all its leaves are done. The same
title in two initiatives is allowed, and a reference to it is `ambiguous`, listing each candidate with its initiative
(`Dup, in One`; in the details, `initiative` is `{id, title}`, or null in no initiative); one shared by unfinished and
finished work names the unfinished. An import that would clash is refused as `invalid-import`, naming both items, the
title and the initiative. Replicas that made one title apart leave a `duplicate-title` on `acyclic needs`.
Never put counters in titles.

`acyclic item` also has `reopen` and `delete`; `delete` refuses an item with children, anything it waits
for or that waits for it, evidence, a done state or someone else's live claim on its thread. `acyclic import <file>`
applies a whole plan document, its records named by ids from `acyclic id`, and `--dry-run` reports what it would
change. `acyclic import --schema` prints the plan format. `acyclic thread delete` deletes a thread no item is in and
no one holds a live claim on.

Every item belongs to an initiative, directly or through its parent: an initiative is a long-lived body of work, a
record of its own (`acyclic initiative add|edit|show|list|delete`), and an item under it is an epic once it has
children, and a task otherwise; anything deeper than an epic is a task. `item add` requires a parent, named with
`--parent` by title, id or the end of an id, and an initiative may be named as an item may. Left out, it goes under the
tracker's only initiative; with none or several the add is refused as `parent-required`, naming the choices. No flag
clears a parent: an item moves under another. An item with no parent, which an older version made, reads as in no
initiative, and `acyclic needs` lists it as `parent-missing`, as it does an item whose parent was deleted. An initiative
is deleted only when no item is under it, and its id is never used again;
`acyclic thread ready --initiative <INITIATIVE>` lists the threads with a task under it. An import
(`acyclic import --schema`) needs a parent on every new item.

Before adding work, read the plan with `acyclic initiative list`, `acyclic item list` and `acyclic thread list`, and
give the item the parent, waits-for and thread it belongs with.

## Output

- `prime`, `id`, `completions`, `import --schema`, and help and version text print documents or text. Every other
  command prints one JSON document when its output is piped, as a tool's is: `{"ok": true, "data": …}` or
  `{"ok": false, "error": {"code", "message", "details"}}`, errors included. Branch on `error.code`, not on the message.
  At a terminal it prints text for people instead; if your commands run in one, pass `-o json`.
- `serve` and `up` print a JSON startup result even at a terminal, then keep running until stopped.
- A bare group such as `acyclic item` prints its help.
- A command line that doesn't parse prints its error as text on standard error, with exit code 2; run
  `acyclic <command> --help` to see the command's arguments. `acyclic help <command>` shows them too.
- Records carry their own id as `id`. `list` and `show` return the same rows, and `show` adds details. Entries in
  `needs` embed the rows they are about.
- Times are RFC 3339 in UTC, such as `2026-10-15T09:30:00Z`.
- Exit codes: 0 done, 1 refused, 2 usage, 3 not found, 4 store (a lock, a lost race or git; retry later),
  5 not set up, 6 unreadable state (run `acyclic doctor`).
- Writes report `changed`. A repeated command changes nothing, except `thread claim`, which renews its lease. Any write
  can also commit a lease renewal that its own `changed` leaves out.
- A `clock-jump` refusal means this replica's saved time is more than five minutes ahead of the system clock. Correct
  the clock and wait until it is within five minutes of the saved time. Writes remain stamped at the saved time until
  the clock catches up fully. If this clone has a shared remote, sync it and continue from a new clone with an unused
  replica id. That migration keeps old timestamps: live claims keep
  their expiry until they expire or their holder releases them.

## Finding and doing work

1. `acyclic thread ready` lists the threads you may work: yours first, then those nobody holds, each group by id in
   natural order (`t2` before `t10`). Each row is the thread's summary with its ready `items`, by id too. An item in no
   thread is a thread of one: it is ready and claimed as its own thread, which `acyclic thread claim <item>` makes,
   by the item's id or title, and a row shows it with `task` set to the item's id and the item's title. Its thread's
   id is the item's id and `.one`. `acyclic item list` lists every item.
   `acyclic item show <item>...` explains items, as an array in the order given: their readiness, children and their
   thread's claims.
2. `acyclic thread claim <thread>` before starting. A claim holds a time lease (60 minutes by default,
   `--lease <minutes>` up to 480). Your claim lapses after its lease unless you keep writing to the tracker: any write
   through this clone's CLI renews a claim of this clone's with less than half of 60 minutes left, if its holder has
   written something other than a claim in the last 60 minutes, counting that write itself. Renewal resets the lease to
   60 minutes, regardless of the initial `--lease`. A write that releases the claim, finishes the thread's last item or
   fails renews nothing, and `sync` doesn't renew. Finish each item as you go, and when you spend long on code without
   writing to the tracker, run `acyclic thread claim <thread>` again as a heartbeat: it renews your claim. A thread
   whose claims have all lapsed while it has ready work is stalled: `thread ready` lists it, marked `stalled`, among
   those nobody holds, `acyclic needs` lists it with its last holder, and anyone may claim it. Don't use `--force` to
   take someone else's work. If `thread claim` or `acyclic thread show <thread>` says another holder wins a contest for
   your thread, or `thread ready` stops listing a thread you hold, stop and run `acyclic thread release <thread>`: it
   drops only your claim. A lapsed lease frees your thread, so after a pause check that `acyclic thread show <thread>`
   still names you as holder.
3. `acyclic item done <item> --commit <owner/name>@<sha>` once its pull request has merged, naming the commit that
   landed on the default branch (the squash commit), or `--evidence` when the result isn't a commit. See "Details". It
   finishes only what your thread holds, or what is ready, and a parent once its children are done; don't use `--force`
   to finish someone else's work. A parent needs no `item done`: its `stage` reads done once every leaf under it is,
   and work waiting on it waits on those leaves. `acyclic thread release <thread>` once none of its items is left ready.
4. `acyclic thread` and `acyclic repository` keep threads and repository records; an item's thread is set with
   `acyclic item edit --thread`. `acyclic thread list` lists the threads with their done and total leaves, and
   `acyclic thread show <thread>` lists each member with its state and readiness. Ready work is ordered by id; when one
   item needs another, make it wait for that one. `thread list` also puts the threads with work left in `waves`, first
   to last, each after the threads its work waits on (a row's `waits_for`), with those that wait on none and that none
   waits on under `anytime`.
5. Grouping work into threads is yours: create threads with `acyclic thread add`, fill them with
   `acyclic item edit --thread`, and give each a title and notes. A new child joins the thread its siblings share,
   finished ones included, and none above it; an add or an import reports it as `placed`. When its siblings differ or
   some are in none, `item add` is refused (`thread-unclear`) unless it gets `--thread` or `--no-thread`, and an import
   unless the new item names its thread; an explicit `--thread` always wins. `item add --claim` claims the item's
   thread in the same write, including the thread of one for a loose item, and cannot be combined with `--no-thread`.
   To add an item outside its inherited thread and then claim it, use `acyclic item add --no-thread`, then
   `acyclic thread claim <item-id>` with the new item's id. A move never changes a thread. An
   item's first child takes its thread, or, when the child is moved in or given its own thread, the parent's is dropped;
   a thread on an item with children is ignored, and `acyclic needs` lists it as `thread-on-parent`, and a thread with
   no item as `empty-thread`. A task's own thread of one is never deleted: it goes from the list once nothing is in it
   and no one holds it. Threads whose work waits on each other can't all finish: `acyclic needs` lists them as a
   `thread-cycle`, and a waits-for or a member should move.
6. `acyclic sync` exchanges work with the tracker's shared repository (`origin`): it fetches every replica's state,
   joins it, and pushes yours; `changed` says whether joining changed this clone and `pushed` counts the refs sent. Run
   it before reading what another machine wrote and after writing what one should see. With
   `git config acyclic.pushAfterWrite true` in the tracker clone, every write pushes too and reports `push`; a push that
   fails leaves the write in place and shows in `acyclic needs` until a sync.
7. `acyclic compact` squashes this replica's local state history when it has more than one commit. It preserves the
   full audit history; a later `acyclic sync` propagates the local squash to peers. After a successful exchange, sync
   suggests compaction once the local state history reaches 1,000 first-parent commits. Compaction is explicit and
   local: it does not sync or push, does not run in the background, and may not reclaim disk space immediately.

## Details

Done means merged: work waiting on a done item may start at once. So a commit is checked when it is written, by
`item done --commit`, `import` and any other write that records one: it must be on the repository's default branch in
this machine's clone of it (named in `~/.config/acyclic/clones.toml`, by the repository's `owner/name`). One not there
yet is fetched first, so a pull request merged a moment ago passes; a branch's commit, or one no clone knows, is refused
as `unmerged`, `--force` or not, and nothing is written. With no clone mapped, no single default branch, or a valid
mapped clone with neither an `origin` remote nor refs for that branch, the write goes through and its result lists the
commit under `unchecked_commits`; `acyclic doctor` later checks every recorded commit, as last fetched, on a machine
that has the clones.

## SSH transport

For SSH remotes, Acyclic uses Gitoxide's non-interactive SSH policy: OpenSSH commands receive `-oBatchMode=yes` before
caller options, and Plink and TortoisePlink receive `-batch`. These are the supported variants `ssh`, `plink` and
`tortoiseplink`, selected with `ssh.variant`. An unknown SSH wrapper must set `ssh.variant` to one of those values and
forward arguments directly to that client; a wrapper's behavior cannot be inferred. The generic `simple` variant and
PuTTY (`putty`) are not supported.

SSH command strings are parsed as literal arguments. Shell operators, substitutions, globs and similar shell syntax are
refused even when quoted or escaped; they are never evaluated. A configured command that disallows shell fallback is a
single literal executable path, so spaces and metacharacters in that path are retained.
