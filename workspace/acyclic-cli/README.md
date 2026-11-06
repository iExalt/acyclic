# `acyclic-cli`

The tracker's command-line interface over `acyclic-core`.

## Conventions

Every command's inputs follow these rules, so you can guess a command from its neighbours.

1. **Verbs.** A command that acts on one kind of record sits in its group, as `acyclic <noun> <verb>`:
   `item add|edit|show|list|done|reopen|delete`, `thread add|edit|show|list|delete|ready|claim|release`,
   and `repository add|edit|show|list`. Only commands that act on the tracker or the tool sit at the
   top: `needs`, `init`, `sync`, `import`, `doctor`, `prime`, `id`, `serve`, `up` and `completions`. Actions that
   aren't a field's value are verbs: `item reopen`, `item delete`, `thread delete`.
2. **Positionals** are only the subject, named for the record: `<ITEM>`, `<THREAD>`, `<REPOSITORY>`. Read
   commands take one or more and answer as an array in the order given; write commands take one. The exceptions are
   the path of `init <PATH>`, the file of `import <FILE>` and the count of `id [COUNT]`.
3. **Every other input is a long flag named for the field it sets.** A scalar field takes `--field <VALUE>` to set it
   and `--no-field` to clear it. A set takes `--field <VALUE>`, repeatable, to add elements and `--no-field <VALUE>`,
   repeatable, to remove them; a removal looks among the elements the record holds first, so it also removes one
   whose record is gone. Naming a field and its `--no-` form together is a usage error. The common flags also
   take a short form, their initial, and the same one in every command that takes them: `-t` for `--title`, `-e` for
   `--evidence`, and `-n` for `--dry-run` as other tools have it; `-o` is `--output`.
4. **`add` and `edit`** take the same fields. `add` creates; adding what exists again with the same values changes
   nothing, and with other values is refused as `already-exists`, naming the fields that differ. `edit` changes an
   existing entity and refuses an unknown one as `not-found`. A write is all or nothing: if any part is refused,
   nothing is written. No add takes a positional: `item add`, `thread add` and `repository add` take `--id <UUIDv7>`,
   and without it the tracker makes one; any other id is refused as `invalid-id`, and `acyclic id [N]` prints new
   ones. An add whose title matches a thread's, for a thread, or an open item's, for an item, is refused as
   `already-exists` with that record's `id` in its details, so a retried add never makes a second record.
5. **Naming a record.** Every item, thread and repository record has a `UUIDv7` id, and every argument that names one
   record takes, looking only among records of its kind: the full id in any case; else an exact title (an item's or a
   thread's) or an `owner/name` in any case (a repository's) that one record has; else the end of one record's id, at
   least 6 characters, in any case. A reference that names several records, such as a title two share or a title of
   one that ends another's id, is refused as `ambiguous` with the `candidates` in its details. An ending shorter than
   6 characters is refused too, as `usage` when one id has it. Text output names items and threads by the shortest
   ending of their ids that names them, and repositories by `owner/name`.
6. **Relations are fields.** An item has `--parent`, `--thread`, `--waits-for` and `--state`. Every actor's change
   applies at once; the actor, `--actor` or `ACYCLIC_ACTOR` and else `actor/user`, only labels the write.
7. **Global options** (`--tracker`, `--actor`, `--output`) are listed under their own heading on every help page.
   `acyclic help` and `acyclic help <command> [<subcommand>]` show a command's help.
8. **Enum-valued inputs** list their values in help, and a wrong value's error lists them again.
9. **Completions.** `acyclic completions <SHELL>` prints a script for bash, zsh, fish, elvish or PowerShell; for zsh,
   `acyclic completions zsh > ~/.zfunc/_acyclic` with `~/.zfunc` on `fpath`.

Waits-for reads left to right: `acyclic item edit 'Plant' --waits-for 'Beds'` says Plant can't start until Beds is done,
and `--no-waits-for 'Beds'` undoes it. `item done <ITEM> --commit <OWNER/NAME>@<SHA>` records the commit that finished
an item, and `--evidence <TEXT>` words. An item's states are `open`, `parked`, `done` and `canceled`; what is under
way is its derived `stage`. A canceled leaf is counted under `canceled` in a `progress`, apart from `leaves`, and never
meets what waits for it: `needs` lists that work as `waits-on-canceled`. Replicas of older versions can't read
`canceled`, so upgrade every replica before canceling. `--state backlog`, the older word for open, is still taken; the
tracker stores open as `backlog`, so replicas of older versions keep reading it.

### Output

Every command's output follows these rules, so you can guess a key from the inputs.

1. **The envelope** is `{"ok": true, "data": …}` or `{"ok": false, "error": {"code", "message", "details"}}`. Every
   command prints one, except `prime`, `id`, `completions`, `import --schema` and help and version text, which print
   text;
   errors are JSON too, except those of a command line that doesn't parse, which print as text on standard error with
   exit code 2. Times are RFC 3339 in UTC and their keys end in `_at`. Pretty output prints the same data for people
   instead: a table for `item list`, `thread ready` and `thread list`, a block of fields for each record, one line for a
   write, and an error as one line on standard error with the same exit code. Tables fit the terminal's width, and print
   without borders or wrapping when piped. Output is pretty at a terminal and JSON when piped, and `-o json|pretty`
   overrides that.
2. **Naming.** A record carries its own id as `id`. A reference is named for the record it points to, in the plural for
   a list: `parent`, `thread`, `item`, `repository`, `repositories`. Keys match the input flags, in snake
   case; enum values are kebab-case. A reference that can name several kinds of record is `{record, id}`, as in a
   conflict, a dangling reference or an unreadable record. An item's `links` read `waits_for`, what it waits for, and
   `waited_on_by`, what waits for it.
3. **One summary per record**, defined once and shared. An item is `{id, title, state, parent, thread, readiness}`,
   its `readiness` tagged by kind as `{readiness: "waiting", waiting}`;
   `thread ready` lists thread summaries, each with its ready `items`. A thread is `{id, title}` and a
   repository record `{id, owner, name, default_branch}`. `list` returns the summaries and `show` returns the same
   object with its details added at the top level, so a thread's `show` adds `members`, each an item summary that says
   whether it is `conflicted`.
4. **Collections are arrays,** and counts sit under `counts`, as in a `progress`, `doctor` and `import`. An absent
   scalar is `null`, an absent set is `[]`, and a flag such as `conflicted` is
   always `true` or `false`. A scalar whose replicas disagree reads `{"conflict": […]}`, and `needs` lists it.
5. **Writes** return `{changed, <record>: id, …}`; `item done` adds the `evidence` it recorded, as
   `{commit: "owner/name@sha", text}`. An item add or an import returns `placed`: the new items that joined the thread
   their siblings, or their parent, held, as `{item, thread}`. A move never changes a thread.
6. **`import`** returns `counts` and `changes`, the records it creates or updates, each with the `fields` that change
   as `{from, to}`: `{record, id, action, fields}`, where `action` is `create` or `update`, or `remove` for a
   waits-for that an item's `waits_for` list left out. A waits-for is a `link` named by `from`, `kind` (`waits-for`)
   and `to`. `--dry-run` reports the same without writing.
7. **`needs`** entries are `{kind, …}`. An entry that is about an item embeds it as its `item list` row under `item`,
   and one about a thread embeds its summary under `thread`; one about several, such as a `thread-cycle`, embeds them
   as a list under `items` or `threads`.
8. **Errors** keep a stable `code`, a one-line message that says what went wrong and, where a command fixes it, names
   that command, and `details` named by the same rules.
