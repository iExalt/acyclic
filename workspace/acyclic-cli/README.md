# `acyclic-cli`

The tracker's command-line interface over `acyclic-core`.

## Conventions

Every command's inputs follow these rules, so you can guess a command from its neighbours.

1. **Verbs.** The daily workflow verbs are top level: `ready`, `list`, `show`, `claim`, `release`, `done`, `needs`.
   Everything else is `acyclic <noun> <verb>`. Actions that aren't a field's value are verbs: `item reopen`, `item
   archive`, `item delete`, `thread approve`, `thread revoke`, `proposal keep`, `proposal drop`.
2. **Positionals** are only the subject's ids, named for the entity: `<ITEM>`, `<THREAD>`, `<REPOSITORY>`,
   `<PROPOSAL>`. Read commands take one or more and answer as an array in the order given; write commands take one.
   The exceptions are the path of `init <PATH>`, the file of `import <FILE>`, `level <LEVEL>`, which sets the global
   level, and `link add|remove <FROM> <KIND> <TO>`.
3. **Every other input is a long flag named for the field it sets.** A scalar field takes `--field <VALUE>` to set it
   and `--no-field` to clear it. A set takes `--field <VALUE>`, repeatable, to add elements and `--no-field <VALUE>`,
   repeatable, to remove them. A boolean is a pair of presence flags, such as `--public` and `--private`. Naming a
   field and its `--no-` form together is a usage error. The common flags also take a short form, their initial:
   `-t` for `--title`, `-n` for `--note`, and `-n` for `--dry-run` as other tools have it. `-o` is left free.
4. **`add` and `edit`** take the same fields. `add` creates; adding what exists again with the same values changes
   nothing, and with other values is refused as `already-exists`, naming the fields that differ. `edit` changes an
   existing entity and refuses an unknown one as `not-found`. A write is all or nothing: if any part is refused,
   nothing is written. `item add` and `thread add` take the id as optional: without one, the tracker assigns
   `<kind>-<replica>-<n>`, one past the highest `n` of that kind, and refuses a retry whose title matches a record's
   as `already-exists` with that record's `id` in its details. A new explicit id starting `item-` or `thread-` is
   refused as `reserved-id`.
5. **Relations are fields.** An item has `--parent`, `--thread`, `--repository` and `--state`; a thread has
   `--repository` and `--level`. An agent's change to an approved thread's membership or repositories doesn't apply:
   it waits as a proposal, listed in the result's `proposals`, for the user to `proposal keep` or `proposal drop`.
6. **Global options** (`--tracker`, `--actor`, `--as-user`, `--output`) are listed under their own heading on every
   help page. `acyclic help` and `acyclic help <command> [<subcommand>]` show a command's help.
7. **Enum-valued inputs** list their values in help, and a wrong value's error lists them again.
8. **Completions.** `acyclic completions <SHELL>` prints a script for bash, zsh, fish, elvish or PowerShell; for zsh,
   `acyclic completions zsh > ~/.zfunc/_acyclic` with `~/.zfunc` on `fpath`.

Links read left to right: `acyclic link add plant waits-for beds` says plant can't start until beds is done. The kinds
are `waits-for`, which holds work back, and `relates-to`, which is context only.

### Output

Every command's output follows these rules, so you can guess a key from the inputs.

1. **The envelope** is `{"ok": true, "data": …}` or `{"ok": false, "error": {"code", "message", "details"}}`. Every
   command prints one, except `prime`, `completions`, `graph` without `--file`, `import --schema` and help and
   version text, which print text; errors are JSON too. Times are RFC 3339 in UTC and their keys end in `_at`.
   Pretty output prints the same data for people instead: a table for `list`, `ready` and `thread list`, a block of
   fields for each record, one line for a write, and an error as one line on standard error with the same exit code.
   Tables fit the terminal's width, and print without borders or wrapping when piped. Output is pretty at a terminal
   and JSON when piped; `ACYCLIC_OUTPUT=json|pretty` changes that default, and `-o json|pretty` overrides both.
2. **Naming.** A record carries its own id as `id`. A reference is named for the entity it points to, in the plural for
   a list: `parent`, `thread`, `item`, `repository`, `proposal`, `repositories`. Keys match the input flags, in snake
   case; enum values are kebab-case. A reference that can name several kinds of entity is `{entity, id}`, as in a
   conflict, a dangling reference or an unreadable record. Links read `waits_for` and `relates_to` from an item and
   `waited_on_by` and `related_from` towards it.
3. **One summary per entity**, defined once and shared. An item is `{id, title, state, parent, thread, readiness}`;
   `ready` lists thread summaries, each with its ready `items`, which add `approved`. A thread is
   `{id, title, approved, heads, counts}`, a repository record `{id, owner, name, public, default_branch}` and a
   proposal `{id, actor, decision, threads}`. `list` returns the summaries and `show` returns the same object with
   its details added at the top level, so a thread's
   `show` adds `members`, each an item summary that says whether it is `approved` and `conflicted`. `proposal list`
   returns every proposal, open ones first, as `proposal show` does, with the item its edit names under `item`.
4. **Collections are arrays,** and counts sit under `counts`, as in a thread's summary, `doctor`, `graph --file`
   and `import`. An absent scalar is `null`, an absent set is `[]`, and a flag such as `approved`, `archived`,
   `reopened`, `public` or `conflicted` is always `true` or `false`. A scalar whose replicas disagree reads
   `{"conflict": […]}`, and `needs` lists it.
5. **Approved** is one meaning everywhere: the thread's approval holds, without a conflict, and it saw the item when it
   recorded the members it saw, or recorded none. `ready --approved` lists exactly the rows whose `approved` is true.
   The user's own change to an approved thread's members, a project manager's, and `proposal keep` extend the approval
   to the member; an agent's change that waits as a proposal doesn't.
6. **Writes** return `{changed, <entity>: id, …}`; `done` adds the `evidence` it recorded. An item or thread edit and
   an import also return `proposals`, every edit the write left waiting for the user as `{id, field}`, its own and the
   placements below, each once, where `field` is `thread` or `repositories`; it is always present.
   An item add, edit or import returns `placed`: the items that joined a thread passed down to them, as `{item, thread,
   outcome, proposal}`, with `outcome` `applied` or `proposed`. `proposal keep` and `proposal drop` return the `item`
   and `thread` the edit named and whether the item is `approved` afterwards.
7. **`import`** returns `counts` and `changes`, the records it creates or updates, or only proposes, each with the
   `fields` that change as `{from, to}`: `{entity, id, action, fields}`, where `action` is `create`, `update` or
   `propose`. A link is named by `from`, `kind` and `to`. `--dry-run` reports the same without writing.
8. **`needs`** entries are `{kind, …}`. An entry that is about an item embeds it as its `list` row under `item`, and one
   about a thread embeds its summary under `thread`. A proposal entry embeds the proposal as `proposal show` returns it
   and the item its edit names under `item`, which is `null` for an edit of a thread's repositories.
9. **Errors** keep a stable `code`, a one-line message that says what went wrong and, where a command fixes it, names
   that command, and `details` named by the same rules.
