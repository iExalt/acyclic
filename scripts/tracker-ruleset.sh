#!/bin/sh
# Stops a tracker's shared repository from rewriting its audit logs: audit-log and usage refs (everything under
# refs/heads/log/ and refs/heads/usage/) may be created and deleted, never force-pushed, by anyone, admins included.
# State refs stay outside it, since a squash rewrites them. GitHub's patterns need `/**/*` to match every depth.
# Creates the ruleset, or updates it in place when it exists, so running it again is safe. Rulesets need the
# repository's administration: run it with the owner's gh login.
#
#   sh scripts/tracker-ruleset.sh <owner>/<repository>
set -eu
repo=${1:?usage: tracker-ruleset.sh <owner>/<repository>}
name='acyclic: append-only logs'
body=$(cat <<EOF
{
  "name": "$name",
  "target": "branch",
  "enforcement": "active",
  "bypass_actors": [],
  "conditions": {"ref_name": {"include": ["refs/heads/log/**/*", "refs/heads/usage/**/*"], "exclude": []}},
  "rules": [{"type": "non_fast_forward"}]
}
EOF
)
id=$(gh api "repos/$repo/rulesets" --jq ".[] | select(.name == \"$name\") | .id")
if [ -n "$id" ]; then
  printf '%s' "$body" | gh api --method PUT "repos/$repo/rulesets/$id" --input - --jq '"updated ruleset \(.id)"'
else
  printf '%s' "$body" | gh api --method POST "repos/$repo/rulesets" --input - --jq '"created ruleset \(.id)"'
fi
