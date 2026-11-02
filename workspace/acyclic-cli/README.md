# `acyclic-cli`

The tracker's command-line interface over `acyclic-core`.

## Conventions

Every command's inputs follow these rules, so you can guess a command from its neighbours.

1. **Verbs.** A command that acts on one kind of record sits in its group, as `acyclic <noun> <verb>`:
   `item add|edit|show|list|done|reopen|archive|delete`, `thread add|edit|show|list|ready|claim|release|renumber`,
   and `repository add|edit|show|list`. Only commands that act on the tracker or the tool sit at the
   top: `needs`, `init`, `sync`, `import`, `doctor`, `prime`, `serve`, `up` and `completions`. Actions that aren't a
   field's value are verbs: `item reopen`, `item archive`, `item delete`, `thread renumber`.
2. **Positionals** are only the subject's ids, named for the entity: `<ITEM>`, `<THREAD>`, `<REPOSITORY>`. Read
   commands take one or more and answer as an array in the order given; write commands take one. The exceptions are
   the path of `init <PATH>` and the file of `import <FILE>`.
3. **Every other input is a long flag named for the field it sets.** A scalar field takes `--field <VALUE>` to set it
   and `--no-field` to clear it. A set takes `--field <VALUE>`, repeatable, to add elements and `--no-field <VALUE>`,
   repeatable, to remove them; a removal names an element by the exact id the record holds, so it also removes one
   whose record is gone. Naming a field and its `--no-` form together is a usage error. The common flags also
   take a short form, their initial, and the same one in every command that takes them: `-t` for `--title`, `-r` for
   `--repository`, `-e` for `--evidence`, and `-n` for `--dry-run` as other tools have it; `-o` is `--output`.
4. **`add` and `edit`** take the same fields. `add` creates; adding what exists again with the same values changes
   nothing, and with other values is refused as `already-exists`, naming the fields that differ. `edit` changes an
   existing entity and refuses an unknown one as `not-found`. A write is all or nothing: if any part is refused,
   nothing is written. `item add` and `thread add` take the id as optional: without one, the tracker assigns
   `<kind>-<replica>-<n>`, one past the highest `n` among that kind's ids starting `<kind>-`, and refuses a retry
   whose title matches a thread's, for a thread, or an open item's, for an item, as `already-exists` with that record's
   `id` in its details. A new explicit id starting `item-` or `thread-` is refused as `reserved-id`.
5. **Relations are fields.** An item has `--parent`, `--thread`, `--repository`, `--waits-for` and `--state`; a
   thread has `--repository`. Every actor's change applies at once; the actor, `--actor` or `ACYCLIC_ACTOR` and else
   `actor/user`, only labels the write.
6. **Global options** (`--tracker`, `--actor`, `--output`) are listed under their own heading on every help page.
   `acyclic help` and `acyclic help <command> [<subcommand>]` show a command's help.
7. **Enum-valued inputs** list their values in help, and a wrong value's error lists them again.
8. **Completions.** `acyclic completions <SHELL>` prints a script for bash, zsh, fish, elvish or PowerShell; for zsh,
   `acyclic completions zsh > ~/.zfunc/_acyclic` with `~/.zfunc` on `fpath`.

Waits-for reads left to right: `acyclic item edit plant --waits-for beds` says plant can't start until beds is done,
and `--no-waits-for beds` undoes it. An item's states are `backlog`, `parked` and `done`; what is under way is its
derived `stage`.

### Output

Every command's output follows these rules, so you can guess a key from the inputs.

1. **The envelope** is `{"ok": true, "data": …}` or `{"ok": false, "error": {"code", "message", "details"}}`. Every
   command prints one, except `prime`, `completions`, `import --schema` and help and version text, which print text;
   errors are JSON too, except those of a command line that doesn't parse, which print as text on standard error with
   exit code 2. Times are RFC 3339 in UTC and their keys end in `_at`. Pretty output prints the same data for people
   instead: a table for `item list`, `thread ready` and `thread list`, a block of fields for each record, one line for a
   write, and an error as one line on standard error with the same exit code. Tables fit the terminal's width, and print
   without borders or wrapping when piped. Output is pretty at a terminal and JSON when piped, and `-o json|pretty`
   overrides that.
2. **Naming.** A record carries its own id as `id`. A reference is named for the entity it points to, in the plural for
   a list: `parent`, `thread`, `item`, `repository`, `repositories`. Keys match the input flags, in snake
   case; enum values are kebab-case. A reference that can name several kinds of entity is `{entity, id}`, as in a
   conflict, a dangling reference or an unreadable record. An item's `links` read `waits_for`, what it waits for, and
   `waited_on_by`, what waits for it.
3. **One summary per entity**, defined once and shared. An item is `{id, title, state, parent, thread, readiness}`;
   `thread ready` lists thread summaries, each with its ready `items`. A thread is `{id, title, heads, counts}` and a
   repository record `{id, owner, name, default_branch}`. `list` returns the summaries and `show` returns the same
   object with its details added at the top level, so a thread's `show` adds `members`, each an item summary that says
   whether it is `conflicted`.
4. **Collections are arrays,** and counts sit under `counts`, as in a thread's summary, `doctor` and `import`. An absent
   scalar is `null`, an absent set is `[]`, and a flag such as `archived`, `reopened` or `conflicted` is
   always `true` or `false`. A scalar whose replicas disagree reads `{"conflict": […]}`, and `needs` lists it.
5. **Writes** return `{changed, <entity>: id, …}`; `item done` adds the `evidence` it recorded. An item add, edit or
   import returns `placed`: the items that joined a thread passed down to them, as `{item, thread}`.
6. **`import`** returns `counts` and `changes`, the records it creates or updates, each with the `fields` that change
   as `{from, to}`: `{entity, id, action, fields}`, where `action` is `create` or `update`. A waits-for is a `link`
   named by `from`, `kind` (`waits-for`) and `to`. `--dry-run` reports the same without writing.
7. **`needs`** entries are `{kind, …}`. An entry that is about an item embeds it as its `item list` row under `item`,
   and one about a thread embeds its summary under `thread`; one about several, such as a `thread-cycle`, embeds them
   as a list under `items` or `threads`.
8. **Errors** keep a stable `code`, a one-line message that says what went wrong and, where a command fixes it, names
   that command, and `details` named by the same rules.
