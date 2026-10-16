//! What the read commands print: one summary row per entity, defined here once and shared by `list`, `show`, `ready`
//! and `needs`. A row holds an entity's id and what to know before opening it; `show` adds the rest to the same
//! object. Nothing here is stored, so the core's types keep their own shape and the output follows the conventions in
//! the README: records carry their own `id`, references are named for the entity, collections are arrays, counts sit
//! under `counts`, booleans are never null, and times are `*_at`.

use std::collections::BTreeMap;

use acyclic_core::claim::{self, Claim};
use acyclic_core::model::{Item, ItemState, Link, LinkKind, Model, Value};
use acyclic_core::ready::Readiness;
use acyclic_core::scope::{Decision, Proposal, ScopeEdit};
use acyclic_core::state::{Millis, State};
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
    /// The item summary: `{id, title, state, parent, thread, readiness}`. An id the model lacks has only its id.
    pub fn item_row(&self, id: &str) -> Result<Json, CliError> {
        let Some(item) = self.model.items.get(id) else {
            return Ok(json!({
                "id": id, "title": null, "state": null, "parent": null, "thread": null,
                "readiness": null,
            }));
        };
        Ok(json!({
            "id": id,
            "title": item.title,
            "state": state(item),
            "parent": item.parent,
            "thread": item.thread,
            "readiness": self.readiness.of(id)?,
        }))
    }

    /// The rows of the items that are ready, with each one's `rank` and whether its work is `approved`; the same
    /// rows, filtered by their own `approved`, when only approved work is asked for.
    pub fn ready_rows(
        &self,
        ready: &[(String, usize)],
        only_approved: bool,
    ) -> Result<Vec<Json>, CliError> {
        let mut rows = Vec::new();
        for (id, rank) in ready {
            let row = with(
                self.item_row(id)?,
                json!({"rank": rank, "approved": approval(self.model, id).1}),
            );
            if row["approved"] == true || !only_approved {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    /// An item in full: its summary and everything else it holds.
    pub fn item(&self, id: &str, item: &Item) -> Result<Json, CliError> {
        let model = self.model;
        let alive = |holder: &str| model.run_alive(holder);
        let claims =
            claim::live_with(self.state, &acyclic_core::model::item_key(id)?, self.now, &alive)?;
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

    /// A thread in full: its summary, and each member as an item summary that also says whether its work is
    /// `approved` and whether it is `conflicted`, listed under each thread it was put in.
    pub fn thread(&self, id: &str, listed: &[Member<'_>]) -> Result<Json, CliError> {
        let mut members = Vec::new();
        for &Member { item, conflict } in listed {
            members.push(with(
                self.item_row(item)?,
                json!({"approved": approval(self.model, item).1, "conflicted": conflict}),
            ));
        }
        let thread = self.model.threads.get(id);
        Ok(with(
            thread_row(self.model, id, listed),
            json!({
                "brief": thread.map(|t| &t.brief),
                "level": thread.map(|t| &t.autonomy),
                "pm": thread.is_some_and(acyclic_core::model::Thread::is_pm),
                "repositories": self.model.repository_set(id),
                "members": members,
            }),
        ))
    }
}

/// An item's links to and from other items, by the words the commands use.
fn links(all: &[Link], id: &str) -> Json {
    let mut found: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for kind in
        ["waits_for", "follows", "relates_to", "waited_on_by", "followed_by", "related_from"]
    {
        found.insert(kind, Vec::new());
    }
    for link in all {
        let (out, back) = match link.kind {
            LinkKind::Blocks => ("waits_for", "waited_on_by"),
            LinkKind::After => ("follows", "followed_by"),
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
pub(crate) fn thread_row(model: &Model, id: &str, members: &[Member<'_>]) -> Json {
    let thread = model.threads.get(id);
    let done = members.iter().filter(|m| model.items[m.item].is_done()).count();
    let conflicted = members.iter().filter(|m| m.conflict).count();
    json!({
        "id": id,
        "title": thread.map(|t| &t.title),
        "approved": thread.is_some_and(|t| t.approved.one() == Some(&true)),
        "heads": model.heads(id),
        "counts": {"members": members.len(), "done": done, "conflicted": conflicted},
    })
}

/// Every thread's summary, in id order.
pub(crate) fn thread_rows(model: &Model) -> Vec<Json> {
    thread_members(model).into_iter().map(|(id, members)| thread_row(model, id, &members)).collect()
}

/// The repository summary: `{id, owner, name, public, default_branch}`.
pub(crate) fn repo_row(record: &acyclic_core::model::Repository) -> Json {
    json!({
        "id": record.id,
        "owner": record.owner,
        "name": record.name,
        "public": record.public.one() == Some(&true),
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
    /// and following `b`, `d` done with evidence and archived, and a repository `r` on `a`; `a` is claimed.
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
        link(&mut r, "c", "b", LinkKind::After, user).unwrap();
        let repo = key(kind::REPOSITORY, "r");
        for (field, value) in [
            (repository::OWNER, json!("me")),
            (repository::NAME, json!("code")),
            (repository::DEFAULT_BRANCH, json!("main")),
        ] {
            model::set(&mut r, &repo, field, Some(&value), user).unwrap();
        }
        model::set(&mut r, &repo, repository::PUBLIC, Some(&true), user).unwrap();
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
        claim::claim(&mut r, &key(kind::ITEM, "a"), "session/pi", NOW, Some(60_000), 480 * 60_000)
            .unwrap();
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
                    if ["approved", "archived", "reopened", "public", "conflicted", "pm"]
                        .contains(&name.as_str())
                        && !inner.is_object()
                    {
                        assert!(inner.is_boolean(), "{here}: {inner}");
                    }
                    if inner.is_i64() && name != "rank" {
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
                       "readiness": {"status": "blocked", "waiting": {"a": {"why": "open"}}}})
            );
            let shown = view.item("a", &view.model.items["a"]).unwrap();
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
            assert_eq!(
                view.item("c", &view.model.items["c"]).unwrap()["links"]["follows"],
                json!(["b"])
            );
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
                        for (name, inner) in map {
                            if inner.is_i64() && name != "rank" && name != "members" {
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
            for id in ["a", "d"] {
                walk(&view.item(id, &view.model.items[id]).unwrap(), &mut keys);
            }
        });
        walk(&json!(Reason::Waiting { until: 3 }), &mut keys);
        let expected: BTreeSet<String> = TIMESTAMP_KEYS.map(String::from).into();
        assert_eq!(keys, expected);
    }

    #[test]
    fn a_ready_row_is_approved_exactly_when_ready_approved_lists_it() {
        let r = tracker();
        viewing(&r, |view| {
            let ready: Vec<(String, usize)> =
                ["a", "b", "c", "late"].map(|id| (id.to_owned(), 0)).to_vec();
            let all = view.ready_rows(&ready, false).unwrap();
            let approved = view.ready_rows(&ready, true).unwrap();
            let flags: Vec<(&str, bool)> = all
                .iter()
                .map(|row| (row["id"].as_str().unwrap(), row["approved"].as_bool().unwrap()))
                .collect();
            // The agent's raced add was never seen by the approval, and c has no thread.
            assert_eq!(flags, [("a", true), ("b", true), ("c", false), ("late", false)]);
            assert!(approved.iter().all(|row| row["approved"] == true), "{approved:?}");
            assert_eq!(approved.len(), 2);
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
            let row = thread_row(view.model, "t", &listed["t"]);
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
        assert_eq!(thread_row(&model, "t", &[])["approved"], false, "a conflict isn't an approval");
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
            json!({"id": "r", "owner": "me", "name": "code", "public": true, "default_branch": "main",
                   "installation": null, "items": ["b"], "threads": []})
        );
        follows_the_conventions(&shown, "repository");
    }
}
