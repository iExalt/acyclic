//! What the read commands print: one summary row per entity, defined here once and shared by each group's `list`
//! and `show`, `thread ready` and `needs`. A row holds an entity's id and what to know before opening it; `show` adds the rest to the same
//! object. Nothing here is stored, so the core's types keep their own shape and the output follows the conventions in
//! the README: records carry their own `id`, references are named for the entity, collections are arrays, counts sit
//! under `counts`, booleans are never null, and times are `*_at`.

use std::collections::BTreeMap;

use acyclic_core::claim::{self, Claim};
use acyclic_core::model::{Heads, Item, ItemState, Link, Model, Value};
use acyclic_core::needs::Entry;
use acyclic_core::ready::{Readiness, ReadyThread};
use acyclic_core::state::{Key, Millis, State};
use acyclic_core::time::rfc3339;
use serde_json::{Map, Value as Json, json};

use crate::commands::{Member, thread_members};
use crate::error::CliError;
use crate::resolve::repository_name;

/// Adds `extra`'s keys to the object `row`.
fn with(mut row: Json, extra: Json) -> Json {
    if let (Json::Object(row), Json::Object(extra)) = (&mut row, extra) {
        row.extend(extra);
    }
    row
}

/// An item's state, which reads as backlog until one is set.
fn state(item: &Item) -> Json {
    match &item.state {
        Value::None => json!(ItemState::Backlog),
        other => json!(other),
    }
}

/// Evidence with both parts named, a part nobody gave as `null`: the commit as `owner/name@sha`, by its repository's
/// name as it is now, and the words. A conflict is `{"conflict": [...]}`, each side in that shape, or `null` for a side
/// that cleared it.
pub(crate) fn evidence(model: &Model, value: &Value<acyclic_core::model::Evidence>) -> Json {
    let one = |e: &acyclic_core::model::Evidence| {
        let commit = e.commit.as_ref().map(|sha| {
            let repository = e.repository.as_ref().map(|id| {
                model.repositories.get(id).and_then(repository_name).unwrap_or_else(|| id.clone())
            });
            repository.map_or_else(|| sha.clone(), |repository| format!("{repository}@{sha}"))
        });
        json!({"commit": commit, "text": e.text})
    };
    match value {
        Value::None => Json::Null,
        Value::One(e) => one(e),
        Value::Conflict(sides) => {
            json!({"conflict": sides.iter().map(|side| side.as_ref().map_or(Json::Null, one)).collect::<Vec<_>>()})
        },
    }
}

/// A claim as it prints.
pub(crate) fn claim_json(claim: &Claim) -> Json {
    json!({
        "holder": claim.holder,
        "replica": claim.replica,
        "lease_started_at": rfc3339(claim.lease_start),
        "lease_expires_at": claim.lease_expires.map(rfc3339),
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
    /// item row.
    pub fn ready_rows(&self, ready: &[ReadyThread]) -> Result<Vec<Json>, CliError> {
        let members = thread_members(self.model);
        let heads = self.model.heads_index();
        let mut rows = Vec::new();
        for ReadyThread { thread, items } in ready {
            let found = items.iter().map(|id| self.item_row(id)).collect::<Result<Vec<_>, _>>()?;
            let listed = members.get(thread.as_str()).map_or(&[][..], Vec::as_slice);
            rows.push(with(thread_row(&heads, thread, listed), json!({"items": found})));
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
                "size": item.size,
                "repositories": item.repositories,
                "children": model.children(id),
                "links": links(&model.links, id),
                "claims": claims,
                "evidence": evidence(model, &item.evidence),
                "done_at": item.done_at.map(rfc3339),
                "archived": model.is_archived(id),
                "reopened": item.reopened != Value::None,
            }),
        ))
    }

    /// A thread in full: its summary, its `readiness` and live `claims`, and each member as an item summary that also
    /// says whether it is `conflicted`, listed under each thread it was put in.
    pub fn thread(&self, id: &str, listed: &[Member<'_>]) -> Result<Json, CliError> {
        let claims = claim::live(self.state, &claim::thread_key(id)?, self.now)?;
        let claims: Vec<Json> = claims.iter().map(claim_json).collect();
        let mut members = Vec::new();
        for &Member { item, conflict } in listed {
            members.push(with(self.item_row(item)?, json!({"conflicted": conflict})));
        }
        let thread = self.model.threads.get(id);
        Ok(with(
            thread_row(&self.model.heads_index(), id, listed),
            json!({
                "readiness": self.readiness.thread(id)?,
                "claims": claims,
                "brief": thread.map(|t| &t.brief),
                "repositories": self.model.repository_set(id),
                "members": members,
            }),
        ))
    }
}

impl View<'_> {
    /// The needs-you list: each entry is `{kind, notify, …}`, with item and thread entries embedding their summaries,
    /// and a conflict listing its `values` and how to `resolve` it.
    /// `notify` says whether the user's devices are told of it.
    pub fn needs(&self, entries: &[Entry]) -> Result<Vec<Json>, CliError> {
        let members = thread_members(self.model);
        let heads = self.model.heads_index();
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
                Entry::Unpushed { remote } => json!({"kind": "unpushed", "remote": remote}),
                Entry::LinkCycle { items: ids } => {
                    json!({"kind": "link-cycle", "items": items(ids)?})
                },
                Entry::ParentCycle { items: ids } => {
                    json!({"kind": "parent-cycle", "items": items(ids)?})
                },
                Entry::ThreadCycle { threads } => json!({
                    "kind": "thread-cycle",
                    "threads": threads.iter().map(|id| thread(id)).collect::<Vec<_>>(),
                }),
                Entry::Contest { thread: id, holders } => {
                    json!({"kind": "contest", "thread": thread(id), "holders": holders})
                },
                Entry::ClockAhead { thread: id, holder } => {
                    json!({"kind": "clock-ahead", "thread": thread(id), "holder": holder})
                },
                Entry::Unthreaded { item, fix } => {
                    json!({"kind": "unthreaded", "item": self.item_row(item)?, "fix": fix})
                },
                Entry::Reopened { item, dependents } => json!({
                    "kind": "reopened", "item": self.item_row(item)?, "dependents": items(dependents)?,
                }),
                Entry::DoneEarly { item, open } => {
                    json!({"kind": "done-early", "item": self.item_row(item)?, "open": items(open)?})
                },
                Entry::Dangling { kind, id, field, target, fix } => {
                    let mut shown = json!({
                        "kind": "dangling", "entity": kind, "id": id, "field": field, "target": target,
                    });
                    if let Some(fix) = fix {
                        shown["fix"] = json!(fix);
                    }
                    // An item's reference brings its row, as a conflict's does, so the app can open it.
                    if kind == "item" && self.model.items.contains_key(id.as_str()) {
                        shown["item"] = self.item_row(id)?;
                    }
                    shown
                },
                Entry::Unverified { item, repository, reason } => json!({
                    "kind": "unverified", "item": self.item_row(item)?, "repository": repository,
                    "repository_name": self.model.repositories.get(repository)
                        .and_then(crate::resolve::repository_name),
                    "reason": reason,
                }),
            };
            shown["notify"] = entry.notifies().into();
            found.push(shown);
        }
        Ok(found)
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
            _ => "repository edit",
        };
        Some(format!(
            "acyclic {command} {id} --{flag} <{}>{clear}",
            flag.to_uppercase().replace('-', "_")
        ))
    };
    match (entity, field) {
        ("item" | "thread", "title") => edit(field, false),
        ("repository", "owner" | "name") => edit("name", false),
        ("item", "state") => edit("state", false),
        ("item", "notes" | "size" | "parent" | "thread") | ("thread", "brief") => edit(field, true),
        ("repository", "default-branch") => edit("default-branch", false),
        ("item", "evidence") => Some(format!(
            "acyclic item done {id} --force --commit <REPOSITORY>@<SHA> (or --evidence <TEXT>)"
        )),
        ("item", "archived") => Some(format!("acyclic item archive {id}")),
        ("item", "reopened") => Some(format!("acyclic item reopen {id}")),
        ("link", _) => {
            let read = |name: &str| {
                let key = Key::new("link", id).ok()?;
                let found = state.field(&key, name)?.iter().next()?.1.value.clone();
                found.as_str().map(str::to_owned)
            };
            let (from, to) = (read("from")?, read("to")?);
            Some(format!("acyclic item edit {from} --waits-for|--no-waits-for {to}"))
        },
        _ => None,
    }
}

/// What an item waits for and what waits for it.
fn links(all: &[Link], id: &str) -> Json {
    let mut found: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let (out, back) = ("waits_for", "waited_on_by");
    for kind in [out, back] {
        found.insert(kind, Vec::new());
    }
    for link in all {
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

/// The thread summary: `{id, title, heads, counts: {members, done, conflicted}}`. A thread only an item names has no
/// title.
pub(crate) fn thread_row(heads: &Heads<'_>, id: &str, members: &[Member<'_>]) -> Json {
    let model = heads.model();
    let thread = model.threads.get(id);
    let done = members.iter().filter(|m| model.reads_done(m.item)).count();
    let conflicted = members.iter().filter(|m| m.conflict).count();
    json!({
        "id": id,
        "title": thread.map(|t| &t.title),
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

#[cfg(test)]
mod tests {

    use acyclic_core::model::{self, By, add_item, link, set_parent, set_state};
    use acyclic_core::ready::Ancestry;
    use acyclic_core::schema::{kind, repository};
    use acyclic_core::scope::{self, ScopeEdit, USER};
    use acyclic_core::state::{Key, Replica, ReplicaId};

    use super::*;

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

    /// Items `goal` > `a` and `b` in thread `t`, `late` an agent's add to it, `c` waiting for `a`, `d` done with
    /// evidence and archived, and a repository `r` on `a`; `a` is claimed.
    fn tracker() -> Replica {
        let mut r = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
        let user = by(USER);
        for id in ["goal", "a", "b", "c", "d", "late"] {
            add_item(&mut r, id, &id.to_uppercase(), user).unwrap();
        }
        for id in ["a", "b"] {
            set_parent(&mut r, id, Some("goal"), user).unwrap();
        }
        link(&mut r, "c", "a", user).unwrap();
        let repo = key(kind::REPOSITORY, "r");
        for (field, value) in [
            (repository::OWNER, json!("me")),
            (repository::NAME, json!("code")),
            (repository::DEFAULT_BRANCH, json!("main")),
        ] {
            model::set(&mut r, &repo, field, Some(&value), user).unwrap();
        }
        model::set(&mut r, &key(kind::THREAD, "t"), "title", Some(&"T"), user).unwrap();
        scope::edit(&mut r, &member("a", Some("t")), user).unwrap();
        scope::edit(&mut r, &member("b", Some("t")), user).unwrap();
        r.write(&model::item_key("late").unwrap(), "thread", "session/mac", NOW, Some(json!("t")))
            .unwrap();
        let evidence = acyclic_core::model::Evidence {
            repository: Some("r".into()),
            commit: Some("abc1234".into()),
            text: Some("shipped".into()),
        };
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
                    if ["archived", "reopened", "conflicted"].contains(&name.as_str())
                        && !inner.is_object()
                    {
                        assert!(inner.is_boolean(), "{here}: {inner}");
                    }
                    if name.ends_with("_at") && !inner.is_null() {
                        let time = inner.as_str().unwrap_or_default();
                        assert!(
                            time.len() == 20 && time.ends_with('Z'),
                            "{here}: {inner} isn't RFC 3339"
                        );
                    }
                    assert!(
                        !inner.is_i64() || name == "members",
                        "{here}: a number that isn't counted"
                    );
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
            assert_eq!(links, &json!({"waits_for": ["a"], "waited_on_by": []}));
            assert_eq!(shown["claims"][0]["holder"], "session/pi");
            assert_eq!(
                (&shown["archived"], &shown["reopened"], &shown["done_at"]),
                (&json!(false), &json!(false), &Json::Null)
            );
            let done = view.item("d", &view.model.items["d"]).unwrap();
            // The commit by its repository's owner/name, which a rename changes.
            assert_eq!(done["evidence"], json!({"commit": "me/code@abc1234", "text": "shipped"}));
            assert_eq!(done["archived"], true);
            assert!(done["done_at"].is_string());
            for shape in [&row, &shown, &done] {
                follows_the_conventions(shape, "item");
            }
        });
    }

    #[test]
    fn conflicted_evidence_shows_each_side_as_evidence_shows() {
        let r = tracker();
        let model = Model::read(r.state()).unwrap();
        let side = |repository: &str, commit: &str| acyclic_core::model::Evidence {
            repository: Some(repository.into()),
            commit: Some(commit.into()),
            text: None,
        };
        let conflict =
            Value::Conflict(vec![Some(side("r", "abc1234")), Some(side("gone", "def5678")), None]);
        assert_eq!(
            evidence(&model, &conflict),
            json!({"conflict": [{"commit": "me/code@abc1234", "text": null},
                                {"commit": "gone@def5678", "text": null}, null]})
        );
    }

    #[test]
    fn a_ready_row_is_the_threads_summary_with_its_ready_items() {
        let r = tracker();
        viewing(&r, |view| {
            let items = ["a", "b", "late"].map(str::to_owned).to_vec();
            let rows = view.ready_rows(&[ReadyThread { thread: "t".into(), items }]).unwrap();
            let listed = thread_members(view.model);
            let summary = thread_row(&view.model.heads_index(), "t", &listed["t"]);
            for (name, value) in summary.as_object().unwrap() {
                assert_eq!(&rows[0][name], value, "{name}");
            }
            let ids: Vec<&Json> =
                rows[0]["items"].as_array().unwrap().iter().map(|i| &i["id"]).collect();
            assert_eq!(ids, ["a", "b", "late"]);
            assert_eq!(rows[0]["items"][0], view.item_row("a").unwrap());
            follows_the_conventions(&rows[0], "ready");
        });
    }

    #[test]
    fn a_thread_is_one_row_that_show_extends_with_its_members() {
        let clean = tracker();
        viewing(&clean, |view| {
            let listed = thread_members(view.model);
            let shown = view.thread("t", &listed["t"]).unwrap();
            assert_eq!(shown["counts"], json!({"members": 3, "done": 0, "conflicted": 0}));
            let members: Vec<&Json> =
                shown["members"].as_array().unwrap().iter().map(|m| &m["id"]).collect();
            assert_eq!(members, ["a", "b", "late"]);
            assert_eq!(shown["members"][2]["conflicted"], false);
            let row = thread_row(&view.model.heads_index(), "t", &listed["t"]);
            for (name, value) in row.as_object().unwrap() {
                assert_eq!(&shown[name], value, "{name}");
            }
            follows_the_conventions(&shown, "thread");
        });
    }

    #[test]
    fn the_needs_list_embeds_what_each_entry_is_about() {
        let mut mac = tracker();
        let user = by(USER);
        // `c` in `u` waits for `a` in `t`, and `e` in `t` waits for `c`: the two threads wait on each other.
        model::set(&mut mac, &key(kind::THREAD, "u"), "title", Some(&"U"), user).unwrap();
        scope::edit(&mut mac, &member("c", Some("u")), user).unwrap();
        add_item(&mut mac, "e", "E", user).unwrap();
        scope::edit(&mut mac, &member("e", Some("t")), user).unwrap();
        link(&mut mac, "e", "c", user).unwrap();
        // `h` waits for `f`, done in `r`, and `g`, done in a repository with no record, neither merge confirmed.
        add_item(&mut mac, "h", "H", user).unwrap();
        for (id, repository) in [("f", "r"), ("g", "gone")] {
            add_item(&mut mac, id, &id.to_uppercase(), user).unwrap();
            let evidence = acyclic_core::model::Evidence {
                repository: Some(repository.into()),
                commit: Some("abc1234".into()),
                text: None,
            };
            model::set(&mut mac, &key(kind::ITEM, id), "evidence", Some(&evidence), user).unwrap();
            set_state(&mut mac, id, ItemState::Done, user).unwrap();
            link(&mut mac, "h", id, user).unwrap();
        }
        // Two replicas disagree about an item's parent.
        let mut pi = Replica::open(ReplicaId::new("pi").unwrap(), mac.state().clone());
        set_parent(&mut mac, "b", None, user).unwrap();
        set_parent(&mut pi, "b", Some("a"), user).unwrap();
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
            // A thread cycle embeds each thread's summary.
            let cycle = of("thread-cycle")[0];
            let listed = thread_members(view.model);
            assert_eq!(
                cycle["threads"],
                json!(["t", "u"].map(|id| thread_row(&view.model.heads_index(), id, &listed[id])))
            );
            assert_eq!(cycle["notify"], true);
            // An unconfirmed merge names its repository by id, and by `owner/name` when it has a record.
            let named: Vec<(&Json, &Json, &Json)> = of("unverified")
                .into_iter()
                .map(|n| (&n["item"]["id"], &n["repository"], &n["repository_name"]))
                .collect();
            assert_eq!(
                named,
                [
                    (&json!("f"), &json!("r"), &json!("me/code")),
                    (&json!("g"), &json!("gone"), &json!(null))
                ]
            );
            for entry in &needs {
                assert!(entry["kind"].is_string(), "{entry}");
                follows_the_conventions(entry, "needs");
            }
        });
    }

    #[test]
    fn a_conflicted_link_resolves_in_the_command_lines_words() {
        let mut mac = tracker();
        let link_key = key("link", "blocks-1-a-b");
        for (field, value) in [("from", "a"), ("to", "b"), ("kind", "blocks")] {
            mac.write(&link_key, field, USER, 1, Some(json!(value))).unwrap();
        }
        assert_eq!(
            resolve(mac.state(), "link", "blocks-1-a-b", "present").as_deref(),
            Some("acyclic item edit a --waits-for|--no-waits-for b")
        );
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
