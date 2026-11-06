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
`acyclic item edit --thread` (`--no-thread` takes the item out of its thread). `--state` sets `open` or `parked`;
`item done` sets `done` on a leaf, and `item reopen` makes a done leaf open again; a parent is done once
its leaves are, and is never finished or reopened by hand. What is under way shows in an item's
`stage`, from its thread's claim, not in its state.

Leave out `--id` in `acyclic item add`, `acyclic thread add` and `acyclic repository add`: the tracker makes a UUIDv7
and prints it. To know an id before adding, as a plan written ahead does, take new ones from `acyclic id [N]`, which
prints N of them and needs no tracker, and pass each as `--id`; any other id is refused. If an add fails
`already-exists`, a thread (for `thread add`) or an open item (for `item add`) has that title, most likely from an
earlier try; the error's details give its id. Titles are how people find work, so two open items never share one.
Never put counters in titles.

`acyclic item` also has `reopen` and `delete`; `delete` refuses an item with children, anything it waits
for or that waits for it, evidence, a done state or someone else's live claim on its thread. `acyclic import <file>`
applies a whole plan document, its records named by ids from `acyclic id`, and `--dry-run` reports what it would
change. `acyclic import --schema` prints the plan format. `acyclic thread delete` deletes a thread no item is in and
no one holds a live claim on.

Before adding work, read the plan with `acyclic item list` and `acyclic thread list`, and give the item the parent,
waits-for and thread it belongs with.

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
   thread is never ready, and `acyclic needs` lists it as `unthreaded`. `acyclic item list` lists every item.
   `acyclic item show <item>...` explains items, as an array in the order given: their readiness, children and their
   thread's claims.
2. `acyclic thread claim <thread>` before starting. A claim holds a time lease (60 minutes by default,
   `--lease <minutes>` up to 480). Your claim lapses after its lease unless you keep writing to the tracker: any
   write through this clone's CLI renews a claim of this clone's with less than half of 60 minutes left, if its holder
   has written something other than a claim in the last 60 minutes. Renewal resets the lease to 60 minutes, regardless
   of the initial `--lease`. `sync` doesn't renew. Don't use `--force` to take
   someone else's work. If `thread claim` or `acyclic thread show <thread>` says another holder wins a contest for your
   thread, or `thread ready` stops listing a thread you hold, stop and run `acyclic thread release <thread>`: it drops
   only your claim. A lapsed lease frees your thread, so after a pause check that `acyclic thread show <thread>` still
   names you as holder.
3. `acyclic item done <item> --commit <owner/name>@<sha>` once its pull request has merged, naming the commit that
   landed on the default branch (the squash commit), or `--evidence` when the result isn't a commit. See "Details". It
   finishes only what your thread holds, or what is ready, and a parent once its children are done; don't use `--force`
   to finish someone else's work. A parent needs no `item done`: its `stage` reads done once every leaf under it is,
   and work waiting on it waits on those leaves. `acyclic thread release <thread>` once none of its items is left ready.
4. `acyclic thread` and `acyclic repository` keep threads and repository records; an item's thread is set with
   `acyclic item edit --thread`. `acyclic thread list` lists the threads with their counts, and `acyclic thread show
   <thread>` lists each member with its state and readiness. Both list a thread's `heads`: its open leaves that
   nothing in the thread orders. Two or more mean they may run in any order; when one needs another,
   make it wait for that one. `thread list` also puts the threads with work left in `waves`, first to last, each after
   the threads its work waits on (a row's `waits_for`), with those that wait on none and that none waits on under
   `anytime`.
5. Grouping work into threads is yours: create threads with `acyclic thread add`, fill them with `acyclic item edit
   --thread`, and give each a title and notes. New or moved work joins the thread its parent passes down, or else the
   one its siblings share; an add, a move under another parent or an import reports it as `placed`. Threads whose
   work waits on each other can't all finish: `acyclic needs` lists them as a `thread-cycle`, and a waits-for or a
   member should move.
6. `acyclic sync` exchanges work with the tracker's shared repository (`origin`): it fetches every replica's state,
   joins it, and pushes yours; `changed` says whether joining changed this clone and `pushed` counts the refs sent. Run
   it before reading what another machine wrote and after writing what one should see. With
   `git config acyclic.pushAfterWrite true` in the tracker clone, every write pushes too and reports `push`; a push that
   fails leaves the write in place and shows in `acyclic needs` until a sync.

## Details

Done means merged: work waiting on a done item may start at once. So a commit is checked when it is written, by
`item done --commit`, `import` and any other write that records one: it must be on the repository's default branch in
this machine's clone of it (named in `~/.config/acyclic/clones.toml`, by the repository's `owner/name`). One not there
yet is fetched first, so a pull request merged a moment ago passes; a branch's commit, or one no clone knows, is refused
as `unmerged`, `--force` or not, and nothing is written. With no clone mapped, the write goes through and its result
lists the commit under `unchecked_commits`; `acyclic doctor` later checks every recorded commit, as last fetched, on a
machine that has the clones.
