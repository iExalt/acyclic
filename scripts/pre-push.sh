#!/bin/sh
# The publication guard, run by .githooks/pre-push under `mise exec`, so the pinned Gitleaks runs. Before a push leaves
# the machine it checks every outgoing commit, ref name and annotated tag against the deny-list of private terms in
# ~/.config/acyclic/deny-list and against Gitleaks's default rules, and blocks the push on any match or any error.
# Reports name the object and where the term was found, never the term.
#
# Usage: pre-push.sh <remote> <url>, with Git's ref updates on standard input. Never bypass it: fix the content.
set -eu

remote=${1:?usage: pre-push.sh <remote> <url>}
list=${HOME:?HOME is unset, so the deny-list can\'t be located}/.config/acyclic/deny-list
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
blocked=0

die() {
  echo "pre-push: push blocked: $*" >&2
  exit 2
}

[ -f "$list" ] || die "the deny-list is missing: $list"
# Terms are fixed strings, trimmed, with blank lines and # comments dropped, so neither a stray space nor a blank line
# changes what matches.
sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' "$list" | grep -v -e '^$' -e '^#' >"$tmp/terms" || true
[ -s "$tmp/terms" ] || die "the deny-list holds no terms: $list"

# Gitleaks's built-in rules, passed explicitly with an empty ignore file, so nothing in the pushed tree or the
# environment can replace or relax them.
printf '[extend]\nuseDefault = true\n' >"$tmp/gitleaks.toml"
: >"$tmp/gitleaksignore"
git_dir=$(git rev-parse --absolute-git-dir) || die "git can't find the repository"

# check <file> <object> <where>: blocks the push if the file holds a term.
check() {
  if grep -aiqF -f "$tmp/terms" "$1"; then
    echo "pre-push: $2: $3 matches the deny-list" >&2
    blocked=1
  elif [ $? -gt 1 ]; then
    die "grep failed on $3 of $2"
  fi
}

# gitleaks <args…>: blocks the push on a leak; any other failure is an error.
leaks() {
  set +e
  env -u GITLEAKS_CONFIG -u GITLEAKS_CONFIG_TOML gitleaks "$@" --no-banner --redact --exit-code 10 \
    --config "$tmp/gitleaks.toml" --gitleaks-ignore-path "$tmp/gitleaksignore"
  code=$?
  set -e
  case $code in
    0) ;;
    10)
      echo "pre-push: gitleaks found a secret" >&2
      blocked=1
      ;;
    *) die "gitleaks failed with exit code $code" ;;
  esac
}

zero='^0+$'
sha='^([0-9a-f]{40}|[0-9a-f]{64})$'
while read -r local_ref local_sha remote_ref remote_sha rest; do
  [ -n "$local_ref" ] || continue
  printf '%s\n%s\n' "$local_sha" "$remote_sha" | grep -Eqv "$sha" && die "malformed pre-push input"
  [ -z "$rest" ] || die "malformed pre-push input"
  printf '%s\n' "$local_sha" | grep -Eq "$zero" && continue

  printf '%s\n' "$local_ref" >"$tmp/ref"
  check "$tmp/ref" "the local ref pushing $local_sha" "ref name"
  printf '%s\n' "$remote_ref" >"$tmp/ref"
  check "$tmp/ref" "the remote ref receiving $local_sha" "ref name"
  if [ "$(git cat-file -t "$local_sha")" = tag ]; then
    git cat-file -p "$local_sha" >"$tmp/tag" || die "git can't read tag $local_sha"
    check "$tmp/tag" "$local_sha" "tag"
    # Scanned on its own, since a tag pushed for commits the remote has brings no new commits.
    leaks stdin --ignore-gitleaks-allow <"$tmp/tag"
  fi

  # Commits the remote's tracking refs don't hold; the remote's old tip is excluded only when it exists here, since
  # another clone may have moved it.
  set -- "$local_sha" --not "--remotes=$remote"
  if ! printf '%s\n' "$remote_sha" | grep -Eq "$zero" && git cat-file -e "$remote_sha^{commit}" 2>/dev/null; then
    set -- "$@" "$remote_sha"
  fi
  git rev-list "$@" >"$tmp/commits" || die "git can't list the commits to push"
  [ -s "$tmp/commits" ] || continue
  while read -r commit; do
    git show -s --format='%an%n%ae%n%cn%n%ce' "$commit" >"$tmp/identity" || die "git can't read $commit"
    check "$tmp/identity" "$commit" "author or committer"
    git show -s --format=%B "$commit" >"$tmp/message" || die "git can't read $commit"
    check "$tmp/message" "$commit" "commit message"
    git diff-tree -r -z --root --no-commit-id --no-renames --name-only --diff-filter=AMT \
      --diff-merges=first-parent "$commit" >"$tmp/paths" || die "git can't read $commit"
    check "$tmp/paths" "$commit" "file path"
    git log -1 -p --text --no-color --no-ext-diff --no-renames --diff-merges=first-parent --format= "$commit" \
      >"$tmp/patch" || die "git can't read $commit"
    grep -a '^+' "$tmp/patch" >"$tmp/added" || true
    check "$tmp/added" "$commit" "added content"
  done <"$tmp/commits"
  leaks git --ignore-gitleaks-allow --log-opts "$*" "$git_dir"
done

if [ "$blocked" -ne 0 ]; then
  echo "pre-push: push blocked. Fix the content; never bypass this check." >&2
  exit 1
fi
