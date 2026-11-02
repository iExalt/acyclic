//! What the read commands print: one summary row per entity, defined here once and shared by `list`, `show`, `ready`
//! and `needs`. A row holds an entity's id and what to know before opening it; `show` adds the rest to the same
//! object. Nothing here is stored, so the core's types keep their own shape and the output follows the conventions in
//! the README: records carry their own `id`, references are named for the entity, collections are arrays, counts sit
//! under `counts`, booleans are never null, and times are `*_at`.

use std::collections::BTreeMap;

use acyclic_core::claim::{self, Claim};
use acyclic_core::model::{Heads, Item, ItemState, Link, LinkKind, Model, Value};
use acyclic_core::needs::Entry;
use acyclic_core::ready::{Readiness, ReadyThread};
use acyclic_core::scope::{self, Decision, Proposal, ScopeEdit};
use acyclic_core::state::{Key, Millis, State};
use serde_json::{Map, Value as Json, json};

use crate::commands::{Member, thread_members};
use crate::error::CliError;

/// Adds `extra`'s keys to the object `row`.
fn with(mut row: Json, extra: Json) -> Json {
    if let (Json::Object(row), Json::Object(extra)) = (&mut row, extra) {
        row.extend(extra);
    }
    row
}

/// An item's thread, and whether the user approved its work: the thread's approval holds without a conflict, and the
/// approval saw the item when it recorded what it saw, or recorded nothing to compare against. A conflict counts
/// against, unlike for protecting scope, since this says whether work may start.
pub(crate) fn approval(model: &Model, id: &str) -> (Option<String>, bool) {
    let Some(thread) = model.items.get(id).and_then(|item| item.thread.one()) else {
        return (None, false);
    };
    let approved = model
        .threads
        .get(thread)
        .is_some_and(|t| t.approved.one() == Some(&true) && t.approved_member(id) != Some(false));
    (Some(thread.clone()), approved)
}

/// An item's state, which reads as backlog until one is set.
fn state(item: &Item) -> Json {
    match &item.state {
        Value::None => json!(ItemState::Backlog),
        other => json!(other),
    }
}

/// Evidence with every part named, a part nobody gave as `null`.
pub(crate) fn evidence(value: &Value<acyclic_core::model::Evidence>) -> Json {
    match value {
        Value::One(e) => json!({"repository": e.repository, "commit": e.commit, "note": e.note}),
        other => json!(other),
    }
}

/// A claim as it prints.
pub(crate) fn claim_json(claim: &Claim) -> Json {
    json!({
        "holder": claim.holder,
        "replica": claim.replica,
        "lease_started_at": claim.lease_start,
        "lease_expires_at": claim.lease_expires,
    })
}

/// What the item reads need: the model and state they read and the readiness over them.
pub(crate) struct View<'a> {
    pub state: &'a State,
    pub model: &'a Model,
    pub readiness: &'a Readiness<'a>,
    pub now: Millis,
}

impl View<'_> {
    /// The item summary: `{id, title, state, parent, thread, readiness}`, with a leaf's `stage` unless it is archived,
    /// and a parent's `progress`, `{state, counts}`, from the leaves under it and the claims on their threads. `counts`
    /// has its leaves, those `done`, and every other leaf by stage, so they add up to `leaves`; `in_review` counts the
    /// done ones not merged yet. An id the model lacks has only its id.
    pub fn item_row(&self, id: &str) -> Result<Json, CliError> {
        let Some(item) = self.model.items.get(id) else {
            return Ok(json!({
                "id": id, "title": null, "state": null, "parent": null, "thread": null,
                "readiness": null,
            }));
        };
        let mut row = json!({
            "id": id,
            "title": item.title,
            "state": state(item),
            "parent": item.parent,
            "thread": item.thread,
            "readiness": self.readiness.of(id)?,
        });
        if let Some(stage) = self.readiness.stage(id)? {
            row["stage"] = json!(stage);
        }
        if let Some(rollup) = self.readiness.progress(id)? {
            let (progress, stages) = (rollup.progress, rollup.stages);
            row["progress"] = json!({
                "state": progress.state,
                "counts": {
                    "done": progress.done, "leaves": progress.total, "in_review": stages.in_review,
                    "in_progress": stages.in_progress, "ready": stages.ready, "blocked": stages.blocked,
                    "parked": stages.parked, "unthreaded": stages.unthreaded,
                },
            });
        }
        Ok(row)
    }

    /// One row per thread with ready work, in the order given: the thread's summary and its ready `items`, each an
    /// item row with whether its work is `approved`. When only approved work is asked for, the items are filtered by
    /// their own `approved`, and a thread left with none is dropped.
    pub fn ready_rows(
        &self,
        ready: &[ReadyThread],
        only_approved: bool,
    ) -> Result<Vec<Json>, CliError> {
        let members = thread_members(self.model);
        let heads = self.model.heads_index();
        let mut rows = Vec::new();
        for ReadyThread { thread, items } in ready {
            let mut found = Vec::new();
            for id in items {
                let row = with(self.item_row(id)?, json!({"approved": approval(self.model, id).1}));
                if row["approved"] == true || !only_approved {
                    found.push(row);
                }
            }
            if !found.is_empty() {
                let listed = members.get(thread.as_str()).map_or(&[][..], Vec::as_slice);
                rows.push(with(thread_row(&heads, thread, listed), json!({"items": found})));
            }
        }
        Ok(rows)
    }

    /// An item in full: its summary and everything else it holds.
    pub fn item(&self, id: &str, item: &Item) -> Result<Json, CliError> {
        let model = self.model;
        // An item's claims are its thread's.
        let claims = claim::covering(self.state, model, id, self.now)?;
        let claims: Vec<Json> = claims.iter().map(claim_json).collect();
        Ok(with(
            self.item_row(id)?,
            json!({
                "role": model.role(id),
                "notes": item.notes,
                "gate": item.gate,
                "size": item.size,
                "areas": item.areas,
                "resources": item.resources,
                "repositories": item.repositories,
                "children": model.children(id),
                "links": links(&model.links, id),
                "claims": claims,
                "evidence": evidence(&item.evidence),
                "done_at": item.done_at,
                "archived": model.is_archived(id),
                "reopened": item.reopened != Value::None,
            }),
        ))
    }

    /// A thread in full: its summary, its `readiness` and live `claims`, and each member as an item summary that also
    /// says whether its work is `approved` and whether it is `conflicted`, listed under each thread it was put in.
    pub fn thread(&self, id: &str, listed: &[Member<'_>]) -> Result<Json, CliError> {
        let claims = claim::live(self.state, &claim::thread_key(id)?, self.now)?;
        let claims: Vec<Json> = claims.iter().map(claim_json).collect();
        let mut members = Vec::new();
        for &Member { item, conflict } in listed {
            members.push(with(
                self.item_row(item)?,
                json!({"approved": approval(self.model, item).1, "conflicted": conflict}),
            ));
        }
        let thread = self.model.threads.get(id);
        Ok(with(
            thread_row(&self.model.heads_index(), id, listed),
            json!({
                "readiness": self.readiness.thread(id)?,
                "claims": claims,
                "brief": thread.map(|t| &t.brief),
                "level": thread.map(|t| &t.autonomy),
                "repositories": self.model.repository_set(id),
                "members": members,
            }),
        ))
    }
}

impl View<'_> {
    /// The needs-you list: each entry is `{kind, notify, …}`, with item and thread entries embedding their summaries, a
    /// proposal embedding what `proposal show` returns, and a conflict listing its `values` and how to `resolve` it.
    /// `notify` says whether the user's devices are told of it.
    pub fn needs(&self, entries: &[Entry]) -> Result<Vec<Json>, CliError> {
        let members = thread_members(self.model);
        let heads = self.model.heads_index();
        let proposals = scope::proposals(self.state)?;
        // A thread's row walks its heads, and threads waiting on each other in a cycle name each other, so each row is
        // built once: per mention, a plan of thousands of items took seconds.
        let rows: std::cell::RefCell<BTreeMap<String, Json>> = std::cell::RefCell::default();
        let thread = |id: &str| {
            rows.borrow_mut()
                .entry(id.to_owned())
                .or_insert_with(|| {
                    thread_row(&heads, id, members.get(id).map_or(&[][..], Vec::as_slice))
                })
                .clone()
        };
        let items = |ids: &[String]| -> Result<Json, CliError> {
            Ok(Json::Array(ids.iter().map(|id| self.item_row(id)).collect::<Result<_, _>>()?))
        };
        let mut found = Vec::with_capacity(entries.len());
        for entry in entries {
            let mut shown = match entry {
                Entry::Conflict { kind, id, field } => self.conflict(kind, id, field, &thread)?,
                Entry::AwaitingApproval { thread: id, members: open, waits_with, .. } => json!({
                    "kind": "awaiting-approval", "thread": thread(id),
                    "waits_with": waits_with.iter().map(|id| thread(id)).collect::<Vec<_>>(),
                    "counts": {"open_members": open},
                }),
                Entry::Unpushed { remote } => json!({"kind": "unpushed", "remote": remote}),
                Entry::LinkCycle { items: ids } => {
                    json!({"kind": "link-cycle", "items": items(ids)?})
                },
                Entry::ParentCycle { items: ids } => {
                    json!({"kind": "parent-cycle", "items": items(ids)?})
                },
                Entry::Contest { thread: id, holders } => {
                    json!({"kind": "contest", "thread": thread(id), "holders": holders})
                },
                Entry::ResourceClash { resource, holders } => {
                    let holders: Vec<Json> = holders
                        .iter()
                        .map(|(id, holder)| json!({"thread": thread(id), "holder": holder}))
                        .collect();
                    json!({"kind": "resource-clash", "resource": resource, "holders": holders})
                },
                Entry::ClockAhead { thread: id, holder } => {
                    json!({"kind": "clock-ahead", "thread": thread(id), "holder": holder})
                },
                Entry::Unthreaded { item, fix } => {
                    json!({"kind": "unthreaded", "item": self.item_row(item)?, "fix": fix})
                },
                Entry::Proposal { id, .. } => {
                    let proposal = proposals.get(id);
                    json!({
                        "kind": "proposal",
                        "proposal": proposal.map(|p| proposal_show(self.model, p)),
                        "item": match proposal {
                            Some(p) => self.proposal_item(p)?,
                            None => Json::Null,
                        },
                    })
                },
                Entry::Unapproved { item, thread: id, actor, keep, drop } => json!({
                    "kind": "unapproved", "item": self.item_row(item)?, "thread": thread(id),
                    "actor": actor, "keep": keep, "drop": drop,
                }),
                Entry::NewBlocker { item, thread: id, blocker } => json!({
                    "kind": "new-blocker", "item": self.item_row(item)?, "thread": thread(id),
                    "blocker": self.item_row(blocker)?,
                }),
                Entry::Reopened { item, dependents } => json!({
                    "kind": "reopened", "item": self.item_row(item)?, "dependents": items(dependents)?,
                }),
                Entry::DoneEarly { item, open } => {
                    json!({"kind": "done-early", "item": self.item_row(item)?, "open": items(open)?})
                },
                Entry::Dangling { kind, id, field, target } => json!({
                    "kind": "dangling", "entity": kind, "id": id, "field": field, "target": target,
                }),
                Entry::NumberClash { number, threads, fresh, fix } => json!({
                    "kind": "number-clash", "number": number,
                    "threads": threads.iter().map(|id| thread(id)).collect::<Vec<_>>(),
                    "fresh": fresh, "fix": fix,
                }),
                Entry::Unverified { item, repository, reason } => json!({
                    "kind": "unverified", "item": self.item_row(item)?, "repository": repository,
                    "reason": reason,
                }),
                Entry::Question { id, thread: asking, class, text } => json!({
                    "kind": "question", "id": id, "thread": asking.as_deref().map(thread),
                    "class": class, "text": text,
                }),
            };
            shown["notify"] = entry.notifies().into();
            found.push(shown);
        }
        Ok(found)
    }

    /// The item a proposal's edit names, as every entry about an item shows it, or `null` for an edit of a thread's
    /// repositories, which names none.
    fn proposal_item(&self, proposal: &Proposal) -> Result<Json, CliError> {
        match proposal.edit.one() {
            Some(ScopeEdit::Member { item, .. } | ScopeEdit::ItemRepository { item, .. }) => {
                self.item_row(item)
            },
            _ => Ok(Json::Null),
        }
    }

    /// Every proposal, open ones first and then decided ones, each by id: what `proposal show` returns and the item
    /// its edit names.
    pub fn proposals(&self) -> Result<Vec<Json>, CliError> {
        let all = scope::proposals(self.state)?;
        let (open, decided): (Vec<&Proposal>, Vec<&Proposal>) =
            all.values().partition(|p| p.decision == Value::None);
        open.into_iter()
            .chain(decided)
            .map(|p| {
                Ok(with(proposal_show(self.model, p), json!({"item": self.proposal_item(p)?})))
            })
            .collect()
    }

    /// A conflict: the entity and field, the values written apart (a clear among them as `null`), the entity's
    /// summary when it is an item or thread, and the command that settles it, if any command writes the field.
    fn conflict(
        &self,
        entity: &str,
        id: &str,
        field: &str,
        thread: &dyn Fn(&str) -> Json,
    ) -> Result<Json, CliError> {
        let key = Key::new(entity, id)?;
        let mut values: Vec<&Json> = Vec::new();
        for (_, stamped) in self.state.field(&key, field).into_iter().flatten() {
            if !values.contains(&&stamped.value) {
                values.push(&stamped.value);
            }
        }
        let mut entry = json!({
            "kind": "conflict", "entity": entity, "id": id, "field": field, "values": values,
            "resolve": resolve(self.state, entity, id, field),
        });
        match entity {
            "item" => entry["item"] = self.item_row(id)?,
            "thread" => entry["thread"] = thread(id),
            _ => {},
        }
        Ok(entry)
    }
}

/// The command that settles a conflict in `field` of an entity, with the choice left to fill in; none when no command
/// writes the field.
fn resolve(state: &State, entity: &str, id: &str, field: &str) -> Option<String> {
    let edit = |flag: &str, clearable: bool| {
        let clear = if clearable { format!(" (or --no-{flag})") } else { String::new() };
        let command = match entity {
            "item" => "item edit",
            "thread" => "thread edit",
            _ => "repo edit",
        };
        Some(format!(
            "acyclic {command} {id} --{flag} <{}>{clear}",
            flag.to_uppercase().replace('-', "_")
        ))
    };
    match (entity, field) {
        ("item" | "thread", "title") | ("repository", "owner" | "name") => edit(field, false),
        ("item", "state") => edit("state", false),
        ("item", "notes" | "gate" | "size" | "parent" | "thread") | ("thread", "brief") => {
            edit(field, true)
        },
        ("thread", "autonomy") => edit("level", true),
        ("repository", "default-branch") => edit("default-branch", false),
        ("item", "evidence") => Some(format!(
            "acyclic done {id} --force --repository <REPOSITORY> --commit <COMMIT> (or --note <NOTE>)"
        )),
        ("item", "archived") => Some(format!("acyclic item archive {id}")),
        ("item", "reopened") => Some(format!("acyclic item reopen {id}")),
        ("thread", "approved" | "approved-members") => {
            Some(format!("acyclic thread approve|revoke {id} --as-user"))
        },
        ("settings", _) => Some("acyclic level <LEVEL> --as-user".to_owned()),
        ("link", _) => {
            let read = |name: &str| {
                let key = Key::new("link", id).ok()?;
                let found = state.field(&key, name)?.iter().next()?.1.value.clone();
                found.as_str().map(str::to_owned)
            };
            let (from, to, kind) = (read("from")?, read("to")?, read("kind")?);
            let word = match kind.as_str() {
                "blocks" => "waits-for",
                "relates" => "relates-to",
                _ => return None,
            };
            Some(format!("acyclic link add|remove {from} {word} {to}"))
        },
        _ => None,
    }
}

/// An item's links to and from other items, by the words the commands use.
fn links(all: &[Link], id: &str) -> Json {
    let mut found: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for kind in ["waits_for", "relates_to", "waited_on_by", "related_from"] {
        found.insert(kind, Vec::new());
    }
    for link in all {
        let (out, back) = match link.kind {
            LinkKind::Blocks => ("waits_for", "waited_on_by"),
            LinkKind::Relates => ("relates_to", "related_from"),
        };
        if link.from == id {
            found.entry(out).or_default().push(&link.to);
        }
        if link.to == id {
            found.entry(back).or_default().push(&link.from);
        }
    }
    let mut map = Map::new();
    for (name, mut ids) in found {
        ids.sort_unstable();
        map.insert(name.to_owned(), json!(ids));
    }
    Json::Object(map)
}

/// The thread summary: `{id, title, approved, heads, counts: {members, done, conflicted}}`. A thread only an item
/// names has no title and isn't approved. `approved` is whether the approval holds, so a conflict counts as not.
pub(crate) fn thread_row(heads: &Heads<'_>, id: &str, members: &[Member<'_>]) -> Json {
    let model = heads.model();
    let thread = model.threads.get(id);
    let done = members.iter().filter(|m| model.reads_done(m.item)).count();
    let conflicted = members.iter().filter(|m| m.conflict).count();
    json!({
        "id": id,
        "title": thread.map(|t| &t.title),
        "approved": thread.is_some_and(|t| t.approved.one() == Some(&true)),
        "heads": heads.of(id),
        "counts": {"members": members.len(), "done": done, "conflicted": conflicted},
    })
}

/// Every thread's summary, in id order.
pub(crate) fn thread_rows(model: &Model) -> Vec<Json> {
    let heads = model.heads_index();
    thread_members(model)
        .into_iter()
        .map(|(id, members)| thread_row(&heads, id, &members))
        .collect()
}

/// The repository summary: `{id, owner, name, default_branch}`.
pub(crate) fn repo_row(record: &acyclic_core::model::Repository) -> Json {
    json!({
        "id": record.id,
        "owner": record.owner,
        "name": record.name,
        "default_branch": record.default_branch,
    })
}

/// A repository in full: its summary, its installation, and the items and threads that touch it.
pub(crate) fn repo_show(model: &Model, record: &acyclic_core::model::Repository) -> Json {
    let id = &record.id;
    let items: Vec<&str> = model
        .items
        .values()
        .filter(|item| item.repositories.contains(id))
        .map(|item| item.id.as_str())
        .collect();
    let threads: Vec<&str> = model
        .threads
        .values()
        .filter(|thread| thread.repositories.contains(id))
        .map(|thread| thread.id.as_str())
        .collect();
    with(
        repo_row(record),
        json!({"installation": record.installation, "items": items, "threads": threads}),
    )
}

/// The proposal summary: `{id, actor, decision, threads}`, where `threads` are the approved threads it changes.
pub(crate) fn proposal_row(model: &Model, proposal: &Proposal) -> Json {
    let threads = proposal.edit.one().map(|edit| edit.approved_threads(model)).unwrap_or_default();
    json!({
        "id": proposal.id,
        "actor": proposal.actor,
        "decision": proposal.decision.one().map(|d| match d {
            Decision::Keep => "keep",
            Decision::Drop => "drop",
        }),
        "threads": threads,
    })
}

/// A proposal in full: its summary and the edit, with the value it changes from and to. The value before is what
/// the model holds now, so a decided proposal has none to show.
pub(crate) fn proposal_show(model: &Model, proposal: &Proposal) -> Json {
    with(proposal_row(model, proposal), json!({"edit": proposal_edit(model, proposal)}))
}

fn proposal_edit(model: &Model, proposal: &Proposal) -> Json {
    let Some(edit) = proposal.edit.one() else { return Json::Null };
    let decided = proposal.decision != Value::None;
    let before = |now: Json| if decided { Json::Null } else { now };
    let element = |set: &std::collections::BTreeSet<String>, repository: &str, present: bool| {
        let mut after = set.clone();
        if present {
            after.insert(repository.to_owned());
        } else {
            after.remove(repository);
        }
        (json!(set), json!(after))
    };
    match edit {
        ScopeEdit::Member { item, thread } => json!({
            "kind": "member", "item": item, "field": "thread",
            "before": before(model.items.get(item).map_or(Json::Null, |i| json!(i.thread))),
            "after": thread,
        }),
        ScopeEdit::ItemRepository { item, repository, present } => {
            let set = model.items.get(item).map(|i| i.repositories.clone()).unwrap_or_default();
            let (now, after) = element(&set, repository, *present);
            json!({"kind": "item-repository", "item": item, "repository": repository,
                   "present": present, "field": "repositories", "before": before(now), "after": after})
        },
        ScopeEdit::ThreadRepository { thread, repository, present } => {
            let set = model.threads.get(thread).map(|t| t.repositories.clone()).unwrap_or_default();
            let (now, after) = element(&set, repository, *present);
            json!({"kind": "thread-repository", "thread": thread, "repository": repository,
                   "present": present, "field": "repositories", "before": before(now), "after": after})
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use acyclic_core::archive::Reason;
    use acyclic_core::model::{self, By, add_item, link, set_parent, set_state};
    use acyclic_core::ready::Ancestry;
    use acyclic_core::schema::{kind, repository, thread as thread_field};
    use acyclic_core::scope::{self, USER};
    use acyclic_core::state::{Key, Replica, ReplicaId};

    use super::*;
    use crate::TIMESTAMP_KEYS;

    struct NoAncestry;

    impl Ancestry for NoAncestry {
        fn merged(&self, _: &acyclic_core::model::Repository, _: &str) -> Result<bool, String> {
            Err("none".into())
        }
    }

    const NOW: Millis = 1_000;

    fn by(actor: &str) -> By<'_> {
        By { actor, at: NOW }
    }

    fn key(kind: &str, id: &str) -> Key {
        Key::new(kind, id).unwrap()
    }

    /// Items `goal` > `a` and `b` in approved thread `t`, `late` an agent's raced add to it, `c` waiting for `a`
    /// and relating to `b` through an after link as older versions stored one, `d` done with evidence and archived, and a repository `r` on `a`; `a` is claimed.
    fn tracker() -> Replica {
        let mut r = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
        let user = by(USER);
        for id in ["goal", "a", "b", "c", "d", "late"] {
            add_item(&mut r, id, &id.to_uppercase(), user).unwrap();
        }
        for id in ["a", "b"] {
            set_parent(&mut r, id, Some("goal"), user).unwrap();
        }
        link(&mut r, "c", "a", LinkKind::Blocks, user).unwrap();
        link(&mut r, "c", "b", LinkKind::Relates, user).unwrap();
        let repo = key(kind::REPOSITORY, "r");
        for (field, value) in [
            (repository::OWNER, json!("me")),
            (repository::NAME, json!("code")),
            (repository::DEFAULT_BRANCH, json!("main")),
        ] {
            model::set(&mut r, &repo, field, Some(&value), user).unwrap();
        }
        scope::edit(&mut r, &member("a", Some("t")), user).unwrap();
        scope::edit(&mut r, &member("b", Some("t")), user).unwrap();
        scope::approve(&mut r, "t", true, user).unwrap();
        r.write(&model::item_key("late").unwrap(), "thread", "session/mac", NOW, Some(json!("t")))
            .unwrap();
        let evidence =
            acyclic_core::model::Evidence { note: Some("shipped".into()), ..Default::default() };
        model::set(&mut r, &key(kind::ITEM, "d"), "evidence", Some(&evidence), user).unwrap();
        set_state(&mut r, "d", ItemState::Done, user).unwrap();
        r.write(&key(kind::ITEM, "d"), "archived", USER, NOW, Some(json!(true))).unwrap();
        claim::claim(&mut r, "t", "session/pi", NOW, 60_000, 480 * 60_000).unwrap();
        r
    }

    fn member(item: &str, thread: Option<&str>) -> ScopeEdit {
        ScopeEdit::Member { item: item.to_owned(), thread: thread.map(str::to_owned) }
    }

    /// Runs `check` over a view of `replica`.
    fn viewing<T>(replica: &Replica, check: impl FnOnce(&View<'_>) -> T) -> T {
        let model = Model::read(replica.state()).unwrap();
        let readiness = Readiness::new(replica.state(), &model, NOW, &NoAncestry);
        check(&View { state: replica.state(), model: &model, readiness: &readiness, now: NOW })
    }

    /// The output conventions, for any JSON a command prints: keys in snake case, the adjectives always a boolean,
    /// the times named `*_at`.
    fn follows_the_conventions(value: &Json, path: &str) {
        match value {
            Json::Object(map) => {
                for (name, inner) in map {
                    let here = format!("{path}.{name}");
                    assert!(
                        name.chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                        "{here}: not snake case"
                    );
                    // What sits under `counts` is a number, whatever it is called.
                    if path.ends_with(".counts") {
                        assert!(inner.is_u64(), "{here}");
                        continue;
                    }
                    if ["approved", "archived", "reopened", "conflicted"].contains(&name.as_str())
                        && !inner.is_object()
                    {
                        assert!(inner.is_boolean(), "{here}: {inner}");
                    }
                    if inner.is_i64() {
                        assert!(
                            TIMESTAMP_KEYS.contains(&name.as_str()),
                            "{here}: a number that isn't counted"
                        );
                    }
                    follows_the_conventions(inner, &here);
                }
            },
            Json::Array(values) => values.iter().for_each(|v| follows_the_conventions(v, path)),
            _ => {},
        }
    }

    #[test]
    fn an_item_is_one_row_that_show_extends() {
        let r = tracker();
        viewing(&r, |view| {
            let row = view.item_row("c").unwrap();
            assert_eq!(
                row,
                json!({"id": "c", "title": "C", "state": "backlog", "parent": null, "thread": null,
                       "readiness": {"status": "blocked", "waiting": {"a": {"why": "open"}}}, "stage": "blocked"})
            );
            // A parent's summary adds its progress from its leaves, in progress while their thread is claimed though
            // none is started; a leaf's has none.
            assert_eq!(
                view.item_row("goal").unwrap()["progress"],
                json!({"state": "in-progress", "counts": {"done": 0, "leaves": 2, "in_review": 0, "in_progress": 2,
                       "ready": 0, "blocked": 0, "parked": 0, "unthreaded": 0}})
            );
            let shown = view.item("a", &view.model.items["a"]).unwrap();
            assert_eq!(shown.get("progress"), None);
            // The summary is at the top level, with no nested item, and show adds the rest.
            let summary = view.item_row("a").unwrap();
            for (name, value) in summary.as_object().unwrap() {
                assert_eq!(&shown[name], value, "{name}");
            }
            assert_eq!(shown["links"]["waited_on_by"], json!(["c"]));
            assert_eq!(
                view.item("c", &view.model.items["c"]).unwrap()["links"]["waits_for"],
                json!(["a"])
            );
            let links = &view.item("c", &view.model.items["c"]).unwrap()["links"];
            assert_eq!(links["relates_to"], json!(["b"]));
            assert_eq!(shown["claims"][0]["holder"], "session/pi");
            assert_eq!(
                (&shown["archived"], &shown["reopened"], &shown["done_at"]),
                (&json!(false), &json!(false), &Json::Null)
            );
            let done = view.item("d", &view.model.items["d"]).unwrap();
            assert_eq!(
                done["evidence"],
                json!({"repository": null, "commit": null, "note": "shipped"})
            );
            assert_eq!(done["archived"], true);
            assert!(done["done_at"].is_i64());
            for shape in [&row, &shown, &done] {
                follows_the_conventions(shape, "item");
            }
        });
    }

    #[test]
    fn every_time_in_a_view_is_a_known_key() {
        let r = tracker();
        let mut keys = BTreeSet::new();
        let walk = |value: &Json, keys: &mut BTreeSet<String>| {
            fn go(value: &Json, keys: &mut BTreeSet<String>) {
                match value {
                    Json::Object(map) => {
                        // What sits under `counts` is counted, not a time.
                        for (name, inner) in map.iter().filter(|(name, _)| *name != "counts") {
                            if inner.is_i64() && name != "members" {
                                keys.insert(name.clone());
                            }
                            go(inner, keys);
                        }
                    },
                    Json::Array(values) => values.iter().for_each(|v| go(v, keys)),
                    _ => {},
                }
            }
            go(value, keys);
        };
        viewing(&r, |view| {
            for id in ["a", "d", "goal"] {
                walk(&view.item(id, &view.model.items[id]).unwrap(), &mut keys);
            }
        });
        walk(&json!(Reason::Waiting { until: 3 }), &mut keys);
        let expected: BTreeSet<String> = TIMESTAMP_KEYS.map(String::from).into();
        assert_eq!(keys, expected);
    }

    #[test]
    fn a_ready_row_is_a_thread_whose_items_are_approved_exactly_when_ready_approved_lists_them() {
        let r = tracker();
        viewing(&r, |view| {
            let items = ["a", "b", "late"].map(str::to_owned).to_vec();
            let ready = [ReadyThread { thread: "t".into(), items }];
            let all = view.ready_rows(&ready, false).unwrap();
            let approved = view.ready_rows(&ready, true).unwrap();
            assert_eq!(
                (all[0]["id"].as_str(), all[0]["approved"].as_bool()),
                (Some("t"), Some(true))
            );
            let flags: Vec<(&str, bool)> = all[0]["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| (row["id"].as_str().unwrap(), row["approved"].as_bool().unwrap()))
                .collect();
            // The agent's raced add was never seen by the approval.
            assert_eq!(flags, [("a", true), ("b", true), ("late", false)]);
            let kept = approved[0]["items"].as_array().unwrap();
            assert!(kept.iter().all(|row| row["approved"] == true), "{approved:?}");
            assert_eq!(kept.len(), 2);
            // A thread left with no approved work isn't listed.
            let unapproved = [ReadyThread { thread: "t".into(), items: vec!["late".into()] }];
            assert_eq!(view.ready_rows(&unapproved, true).unwrap(), Vec::<Json>::new());
            for row in &all {
                follows_the_conventions(row, "ready");
            }
        });
    }

    #[test]
    fn a_thread_is_one_row_that_show_extends_with_its_members() {
        let clean = tracker();
        viewing(&clean, |view| {
            let listed = thread_members(view.model);
            let shown = view.thread("t", &listed["t"]).unwrap();
            assert_eq!(shown["counts"], json!({"members": 3, "done": 0, "conflicted": 0}));
            assert_eq!(shown["approved"], true);
            let members: Vec<(&str, bool)> = shown["members"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| (m["id"].as_str().unwrap(), m["approved"].as_bool().unwrap()))
                .collect();
            assert_eq!(members, [("a", true), ("b", true), ("late", false)]);
            assert_eq!(shown["members"][2]["conflicted"], false);
            let row = thread_row(&view.model.heads_index(), "t", &listed["t"]);
            for (name, value) in row.as_object().unwrap() {
                assert_eq!(&shown[name], value, "{name}");
            }
            follows_the_conventions(&shown, "thread");
        });
        let mut conflicted = clean.clone();
        let other = {
            let mut other = Replica::open(ReplicaId::new("pi").unwrap(), clean.state().clone());
            model::set(
                &mut other,
                &key(kind::THREAD, "t"),
                thread_field::APPROVED,
                Some(&false),
                by(USER),
            )
            .unwrap();
            other
        };
        conflicted.join(other.state()).unwrap();
        let model = Model::read(conflicted.state()).unwrap();
        assert_eq!(
            thread_row(&model.heads_index(), "t", &[])["approved"],
            false,
            "a conflict isn't an approval"
        );
        assert_eq!(approval(&model, "a"), (Some("t".to_owned()), false));
    }

    #[test]
    fn a_proposal_shows_what_it_changes_from_and_to_until_it_is_decided() {
        let mut r = tracker();
        let agent = by("session/mac");
        scope::edit(&mut r, &member("c", Some("t")), agent).unwrap();
        scope::edit(&mut r, &member("a", None), agent).unwrap();
        let repository =
            ScopeEdit::ItemRepository { item: "b".into(), repository: "r".into(), present: true };
        scope::edit(&mut r, &repository, agent).unwrap();
        let model = Model::read(r.state()).unwrap();
        let all = scope::proposals(r.state()).unwrap();
        let shown: Vec<Json> = all.values().map(|p| proposal_show(&model, p)).collect();
        let edits: Vec<&Json> = shown.iter().map(|p| &p["edit"]).collect();
        assert!(edits.contains(&&json!({"kind": "member", "item": "c", "field": "thread", "before": null, "after": "t"})));
        assert!(edits.contains(&&json!({"kind": "member", "item": "a", "field": "thread", "before": "t", "after": null})));
        assert!(edits.contains(
            &&json!({"kind": "item-repository", "item": "b", "repository": "r", "present": true,
                                         "field": "repositories", "before": [], "after": ["r"]})
        ));
        assert_eq!(shown[0]["threads"], json!(["t"]));
        assert_eq!(
            (&shown[0]["decision"], &shown[0]["actor"]),
            (&Json::Null, &json!("session/mac"))
        );
        let id = all.keys().next().unwrap().clone();
        scope::keep(&mut r, &id, by(USER)).unwrap();
        let model = Model::read(r.state()).unwrap();
        let decided = proposal_show(&model, &scope::proposals(r.state()).unwrap()[&id]);
        assert_eq!(
            (&decided["decision"], &decided["edit"]["before"]),
            (&json!("keep"), &Json::Null)
        );
        assert_ne!(decided["edit"]["after"], Json::Null);
        for p in shown.iter().chain([&decided]) {
            follows_the_conventions(p, "proposal");
        }
    }

    #[test]
    fn the_needs_list_embeds_what_each_entry_is_about() {
        let mut mac = tracker();
        let agent = by("session/mac");
        scope::edit(&mut mac, &member("c", Some("t")), agent).unwrap();
        let user = by(USER);
        // Two replicas disagree about an item's parent and a thread's approval.
        let mut pi = Replica::open(ReplicaId::new("pi").unwrap(), mac.state().clone());
        set_parent(&mut mac, "b", None, user).unwrap();
        set_parent(&mut pi, "b", Some("a"), user).unwrap();
        model::set(&mut mac, &key(kind::THREAD, "u"), thread_field::APPROVED, Some(&true), user)
            .unwrap();
        model::set(&mut pi, &key(kind::THREAD, "u"), thread_field::APPROVED, Some(&false), user)
            .unwrap();
        mac.join(pi.state()).unwrap();
        viewing(&mac, |view| {
            let entries =
                acyclic_core::needs::needs_you(view.state, view.model, view.readiness, NOW)
                    .unwrap();
            let needs = view.needs(&entries).unwrap();
            let of =
                |kind: &str| -> Vec<&Json> { needs.iter().filter(|n| n["kind"] == kind).collect() };
            let conflicts = of("conflict");
            let parent = conflicts.iter().find(|c| c["field"] == "parent").unwrap();
            assert_eq!(parent["entity"], "item");
            assert_eq!(parent["item"]["id"], "b");
            assert_eq!(parent["values"], json!([null, "a"]));
            assert_eq!(parent["resolve"], "acyclic item edit b --parent <PARENT> (or --no-parent)");
            // A conflicting approval isn't one, and the command that settles it is the user's.
            let approval = conflicts.iter().find(|c| c["field"] == "approved").unwrap();
            assert_eq!(
                (&approval["thread"]["id"], &approval["thread"]["approved"]),
                (&json!("u"), &json!(false))
            );
            assert_eq!(approval["resolve"], "acyclic thread approve|revoke u --as-user");
            // A proposal is what `proposal show` returns, and an unseen member names the thread and its commands.
            let proposal = of("proposal")[0];
            let all = scope::proposals(view.state).unwrap();
            assert_eq!(
                proposal["proposal"],
                proposal_show(view.model, all.values().next().unwrap())
            );
            assert_eq!(proposal["proposal"]["edit"]["after"], "t");
            assert_eq!(
                proposal["item"],
                view.item_row("c").unwrap(),
                "the item, as `list` shows it"
            );
            let unseen = of("unapproved")[0];
            assert_eq!(
                (&unseen["item"]["id"], &unseen["thread"]["id"]),
                (&json!("late"), &json!("t"))
            );
            assert_eq!(unseen["keep"], "acyclic item edit late --thread t");
            for entry in &needs {
                assert!(entry["kind"].is_string(), "{entry}");
                follows_the_conventions(entry, "needs");
            }
        });
    }

    #[test]
    fn a_threads_level_is_settled_by_thread_edit() {
        let state = tracker();
        assert_eq!(
            resolve(state.state(), "thread", "t", "autonomy").as_deref(),
            Some("acyclic thread edit t --level <LEVEL> (or --no-level)")
        );
    }

    #[test]
    fn a_conflicted_link_resolves_in_the_command_lines_words() {
        let mut mac = tracker();
        let link_key = key("link", "relates-1-a-b");
        for (field, value) in [("from", "a"), ("to", "b"), ("kind", "relates")] {
            mac.write(&link_key, field, USER, 1, Some(json!(value))).unwrap();
        }
        assert_eq!(
            resolve(mac.state(), "link", "relates-1-a-b", "present").as_deref(),
            Some("acyclic link add|remove a relates-to b")
        );
    }

    #[test]
    fn proposals_list_open_before_decided_with_the_item_each_names() {
        let mut r = tracker();
        let agent = by("session/mac");
        scope::edit(&mut r, &member("c", Some("t")), agent).unwrap();
        let more = ScopeEdit::ThreadRepository {
            thread: "t".into(),
            repository: "r".into(),
            present: true,
        };
        scope::edit(&mut r, &more, agent).unwrap();
        let first = scope::proposals(r.state()).unwrap().into_keys().next().unwrap();
        scope::keep(&mut r, &first, by(USER)).unwrap();
        viewing(&r, |view| {
            let listed = view.proposals().unwrap();
            // The open one comes first, though it has the later id; the decided one keeps its item.
            let shown: Vec<(&Json, &Json)> =
                listed.iter().map(|p| (&p["decision"], &p["item"]["id"])).collect();
            assert_eq!(shown, [(&Json::Null, &Json::Null), (&json!("keep"), &json!("c"))]);
            assert_eq!(listed[0]["edit"]["kind"], "thread-repository");
            assert_eq!(listed[1]["item"], view.item_row("c").unwrap());
            for p in &listed {
                follows_the_conventions(p, "proposal");
            }
        });
    }

    #[test]
    fn a_repository_is_one_row_that_show_extends_with_what_touches_it() {
        let mut r = tracker();
        scope::edit(
            &mut r,
            &ScopeEdit::ItemRepository { item: "b".into(), repository: "r".into(), present: true },
            by(USER),
        )
        .unwrap();
        let model = Model::read(r.state()).unwrap();
        let shown = repo_show(&model, &model.repositories["r"]);
        assert_eq!(
            shown,
            json!({"id": "r", "owner": "me", "name": "code", "default_branch": "main",
                   "installation": null, "items": ["b"], "threads": []})
        );
        follows_the_conventions(&shown, "repository");
    }
}
