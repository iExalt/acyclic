# `acyclic-cli`

The tracker's command-line interface over `acyclic-core`.

## Conventions

Every command's inputs follow these rules, so a command can be guessed from its neighbours.

1. **Verbs.** The daily workflow verbs are top level: `ready`, `list`, `show`, `claim`, `release`, `done`, `needs`.
   Everything else is `acyclic <noun> <verb>`. Actions that aren't a field's value are verbs: `item reopen`, `item
   archive`, `thread approve`, `thread revoke`, `proposal keep`, `proposal drop`.
2. **Positionals** are only the subject's ids, named for the entity: `<ITEM>`, `<THREAD>`, `<REPOSITORY>`,
   `<PROPOSAL>`. Read commands take one or more and answer as an array in the order given; write commands take one.
   The exceptions are the path of `init <PATH>`, the file of `import <FILE>`, `level <LEVEL>`, which sets the global
   level, and `link add|remove <FROM> <KIND> <TO>`.
3. **Every other input is a long flag named for the field it sets.** A scalar field takes `--field <VALUE>` to set it
   and `--no-field` to clear it. A set takes `--field <VALUE>`, repeatable, to add elements and `--no-field <VALUE>`,
   repeatable, to remove them. A boolean is a pair of presence flags, such as `--public` and `--private`. Naming a
   field and its `--no-` form together is a usage error.
4. **`add` and `edit`** take the same fields. `add` creates; adding what exists again with the same values changes
   nothing, and with other values is refused as `already-exists`, naming the fields that differ. `edit` changes an
   existing entity and refuses an unknown one as `not-found`. A write is all or nothing: if any part is refused,
   nothing is written.
5. **Relations are fields.** An item has `--parent`, `--thread`, `--repository` and `--state`; a thread has
   `--repository` and `--level`. An agent's change to an approved thread's membership or repositories doesn't apply:
   it waits as a proposal, listed in the result's `proposals`, for the user to `proposal keep` or `proposal drop`.
6. **Global options** (`--tracker`, `--actor`, `--as-user`) are listed under their own heading on every help page.
   `acyclic help` and `acyclic help <command> [<subcommand>]` show a command's help.
7. **Enum-valued inputs** list their values in help, and a wrong value's error lists them again.

Links read left to right: `acyclic link add plant waits-for beds` says plant can't start until beds is done. The kinds
are `waits-for` (hard), `follows` (soft) and `relates-to`.
