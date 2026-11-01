# `acyclic-guard`

The publication guard for the public repository. Before a push leaves the machine, it checks every outgoing commit
against a deny-list of private terms and a secret scanner, and blocks the push on any match or error.

## The deny-list

The list lives at `~/.config/acyclic/deny-list`, outside every repository, and is never published. Each non-blank
line is one term; lines starting with `#` are comments. Terms are fixed strings, matched case-insensitively anywhere
in the scanned text. A missing, unreadable or empty list blocks every push.

## What a push is checked for

For each pushed ref (deletions excepted), the guard scans the local and remote ref names and every commit not yet on
the remote's tracking refs:

- author and committer names and emails, and the full message;
- the paths of added, modified and type-changed files;
- every added line of the patch, binary files included, with merges compared against their first parent;
- the text of annotated tags.

Reports name the commit or tag, or for a ref name the local or remote ref, with the channel and the deny-list line that
matched, never the term itself. Gitleaks then scans the same commits with its default rules, ignoring any
`.gitleaks.toml`, `.gitleaksignore` or `gitleaks:allow` in the pushed content; a leak or a Gitleaks error blocks the
push. A push to a URL rather than a named remote has no tracking refs to compare against, so the whole history is
scanned.

Install the hook with `mise run hooks:install`. Never bypass it; if it blocks, fix the content.
