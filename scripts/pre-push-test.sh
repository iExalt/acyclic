#!/bin/sh
# Tests scripts/pre-push.sh as a real pre-push hook: each case pushes from a scratch repository to a bare remote, with a
# synthetic deny-list, and checks the push is blocked or passes. Cases run in parallel; any failure fails the script.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
hook="$root/scripts/pre-push.sh"
base=$(mktemp -d)
trap 'rm -rf "$base"' EXIT INT TERM
term=Forbidden-Term
# Assembled at run time so this file never holds a key-shaped string.
key="AKIA""QYLPMN5HHHFPZAM2"

# setup <dir>: a repository with one clean commit on main, pushed to a bare remote, and a deny-list whose term has a
# trailing space and sits after a blank line and a comment.
setup() {
  dir=$1
  mkdir -p "$dir/home/.config/acyclic"
  printf '# synthetic\n\n%s \n' "$term" >"$dir/home/.config/acyclic/deny-list"
  : >"$dir/home/gitconfig"
  export HOME="$dir/home" GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL="$dir/home/gitconfig"
  export GIT_AUTHOR_NAME="Test Author" GIT_AUTHOR_EMAIL=author@example.com
  export GIT_COMMITTER_NAME="Test Committer" GIT_COMMITTER_EMAIL=committer@example.com
  git init -q --bare "$dir/remote.git"
  git init -q -b main "$dir/work"
  cd "$dir/work"
  git remote add origin "$dir/remote.git"
  printf '#!/bin/sh\nexec sh %s "$@"\n' "'$hook'" >.git/hooks/pre-push
  chmod +x .git/hooks/pre-push
  commit README.md "clean" "docs: start"
  # Seeded without the hook, which the clean case runs, so each case runs Gitleaks only on what it checks.
  git push -q --no-verify origin main
}

commit() {
  mkdir -p "$(dirname "$1")"
  printf '%s\n' "$2" >"$1"
  git add -- "$1"
  git commit -q -m "$3"
}

fail() {
  echo "pre-push-test: $case: $*" >&2
  [ ! -s "$dir/log" ] || sed 's/^/  /' "$dir/log" >&2
  exit 1
}

passes() {
  git push -q "$@" 2>"$dir/log" || fail "push $* was blocked"
}

# blocked <expected> <push args…>: the push fails, saying <expected>, and the guard's report doesn't echo the term.
# Git's own refusal names the refs, so only the guard's lines are checked.
blocked() {
  expected=$1
  shift
  if git push -q "$@" 2>"$dir/log"; then fail "push $* passed"; fi
  grep -qF "$expected" "$dir/log" || fail "push $* didn't say \"$expected\""
  ! grep '^pre-push:' "$dir/log" | grep -qiF "$term" || fail "push $* echoed the term"
}

run_case() {
  case=$1
  setup "$base/$case"
  case $case in
    clean)
      git switch -q -c feature
      commit src/lib.rs "pub fn clean() {}" "feat: add a clean change"
      passes origin feature
      ;;
    added-line)
      commit notes.md "a line naming $term here" "docs: notes"
      blocked "added content matches" origin main
      ;;
    removed-line)
      commit notes.md "$term" "docs: notes"
      passes --no-verify origin main
      git rm -q notes.md
      git commit -q -m "docs: remove notes"
      passes origin main
      ;;
    renamed-file)
      commit old.md "$term" "docs: notes"
      passes --no-verify origin main
      git mv old.md new.md
      git commit -q -m "docs: rename notes"
      blocked "added content matches" origin main
      ;;
    merge)
      git switch -q -c side
      commit side.md "$term" "docs: side"
      passes --no-verify origin side
      git switch -q main
      commit main.md "main" "docs: main"
      git merge -q --no-ff -m "chore: merge side" side
      blocked "added content matches" origin main
      ;;
    path)
      mkdir -p docs
      : >"docs/$term.md"
      git add docs
      git commit -q -m "docs: add a file"
      blocked "file path matches" origin main
      ;;
    binary)
      printf '\000\001\002\000%s\000' "$term" >blob.bin
      git add blob.bin
      git commit -q -m "chore: add a blob"
      blocked "added content matches" origin main
      ;;
    message)
      commit a.txt a "docs: mention

body names $term"
      blocked "commit message matches" origin main
      ;;
    author-email)
      printf 'a\n' >a.txt
      git add a.txt
      git commit -q -m "docs: a" --author "Someone <someone@$term.example>"
      blocked "author or committer matches" origin main
      ;;
    branch-name)
      git switch -q -c "feat/$term"
      blocked "ref name matches" origin "feat/$term"
      ;;
    tag-message)
      git tag -a v0.0.1 -m "release for $term"
      blocked "tag matches" origin refs/tags/v0.0.1
      ;;
    tag-secret)
      git tag -a v0.0.2 -m "aws_access_key_id = $key"
      blocked "gitleaks found a secret" origin refs/tags/v0.0.2
      ;;
    deleting-a-bad-branch)
      git branch "old/$term"
      passes --no-verify origin "old/$term"
      passes origin --delete "old/$term"
      ;;
    missing-list)
      rm "$HOME/.config/acyclic/deny-list"
      commit a.txt a "docs: a"
      blocked "the deny-list is missing" origin main
      ;;
    empty-list)
      printf '# nothing\n\n   \n' >"$HOME/.config/acyclic/deny-list"
      commit a.txt a "docs: a"
      blocked "holds no terms" origin main
      ;;
    secret)
      commit config.env "aws_access_key_id = $key" "chore: config"
      blocked "gitleaks found a secret" origin main
      ;;
    silenced-secret)
      commit config.env "aws_access_key_id = $key # gitleaks:allow" "chore: config"
      fingerprint="$(git rev-parse HEAD):config.env:aws-access-token:1"
      allow_all="[extend]
useDefault = true
[allowlist]
paths = ['.*']
regexes = ['.*']"
      commit .gitleaks.toml "$allow_all" "chore: scanner config"
      commit .gitleaksignore "$fingerprint" "chore: scanner ignores"
      printf '%s\n' "$allow_all" >"$base/$case/allow.toml"
      export GITLEAKS_CONFIG="$base/$case/allow.toml" GITLEAKS_CONFIG_TOML="$allow_all"
      blocked "gitleaks found a secret" origin main
      ;;
    *) fail "no such case" ;;
  esac
}

cases="clean added-line removed-line renamed-file merge path binary message author-email branch-name tag-message
tag-secret deleting-a-bad-branch missing-list empty-list secret silenced-secret"
# Cases named as arguments run alone, such as to time one.
[ $# -eq 0 ] || cases=$*
pids=""
for one in $cases; do
  (run_case "$one") &
  pids="$pids $!"
done
failed=0
count=0
for pid in $pids; do
  wait "$pid" || failed=1
  count=$((count + 1))
done
[ "$failed" -eq 0 ] || exit 1
echo "pre-push-test: $count cases passed"
