//! What the read commands print: one summary row per entity, defined here once and shared by each group's `list`
//! and `show`, `thread ready` and `needs`. A row holds an entity's id and what to know before opening it; `show` adds the rest to the same
//! object. Nothing here is stored, so the core's types keep their own shape and the output follows the conventions in
//! the README: records carry their own `id`, references are named for the entity, collections are arrays, counts sit
//! under `counts`, booleans are never null, and times are `*_at`.

use std::collections::BTreeMap;

use acyclic_core::claim::{self, Claim};
use acyclic_core::model::{Item, ItemState, Link, Model, Value, shown_state};
use acyclic_core::needs::Entry;
use acyclic_core::ready::{ReadinessEvaluator, ReadyThread, Stages};
use acyclic_core::state::{Millis, State};
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

/// An item's state as reads show it, `open` until one is set.
fn state(item: &Item) -> Json {
    match &item.state {
        Value::None => json!(ItemState::Backlog.word()),
        other => shown_state(&json!(other)),
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
    pub readiness: &'a ReadinessEvaluator<'a>,
    pub now: Millis,
}

impl View<'_> {
    /// The item summary: `{id, title, state, parent, thread, readiness}`, with its `stage` (a
    /// parent's rolled up from its leaves and the claims on their threads, none with no leaf to count), and a parent's
    /// `progress`, `{counts}`, from the leaves under it. `counts`
    /// has its leaves, and each leaf under its stage, so the stages add up to `leaves`. An id the model lacks has only its
    /// id.
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
            "thread": self.model.thread_value(item),
            "readiness": self.readiness.of(id)?,
        });
        if let Some(stage) = self.readiness.stage(id)? {
            row["stage"] = json!(stage);
        }
        if let Some(rollup) = self.readiness.progress(id)? {
            row["progress"] = json!({"counts": stage_counts(&rollup.stages)});
            if let Some(stage) = rollup.stage {
                row["stage"] = json!(stage);
            }
        }
        Ok(row)
    }

    /// A thread's progress, `{counts}`: its member leaves by stage, counted as a parent's leaves are.
    pub fn thread_progress(&self, listed: &[Member<'_>]) -> Result<Json, CliError> {
        let stages = Stages::of_leaves(self.readiness, listed.iter().map(|member| member.item))?;
        Ok(json!({"counts": stage_counts(&stages)}))
    }

    /// One row per thread with ready work, in the order given: the thread's summary and its ready `items`, each an
    /// item row.
    pub fn ready_rows(&self, ready: &[ReadyThread]) -> Result<Vec<Json>, CliError> {
        let mut rows = Vec::new();
        for ReadyThread { thread, items, stalled } in ready {
            let found = items.iter().map(|id| self.item_row(id)).collect::<Result<Vec<_>, _>>()?;
            rows.push(with(
                thread_row(self.model, thread),
                json!({"stalled": stalled, "items": found}),
            ));
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
                "children": model.children(id),
                "links": links(&model.links, id),
                "claims": claims,
                "evidence": evidence(model, &item.evidence),
                "done_at": item.done_at.map(rfc3339),
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
            thread_row(self.model, id),
            json!({
                "readiness": self.readiness.thread(id)?,
                "progress": self.thread_progress(listed)?,
                "claims": claims,
                "notes": thread.map(|t| &t.notes),
                "members": members,
            }),
        ))
    }
}

impl View<'_> {
    /// The needs-you list: each entry is `{kind, key, identity, summary, notify, …}`, with item and thread entries embedding
    /// their summaries, and a conflict listing its `values` and how to `resolve` it.
    /// `summary` is the core's one line for it. `notify` says whether the user's devices are told of it.
    pub fn needs(&self, entries: &[Entry]) -> Result<Vec<Json>, CliError> {
        let thread = |id: &str| thread_row(self.model, id);
        let items = |ids: &[String]| -> Result<Json, CliError> {
            Ok(Json::Array(ids.iter().map(|id| self.item_row(id)).collect::<Result<_, _>>()?))
        };
        // Titles as people read them: a title written apart reads as its first value, and an id with none as itself.
        let name = |id: &str| -> String {
            let title = self.model.items.get(id).map(|item| &item.title);
            let title = title.or_else(|| self.model.threads.get(id).map(|thread| &thread.title));
            title
                .and_then(|title| title.all().first().map(|first| (*first).clone()))
                .unwrap_or_else(|| id.to_owned())
        };
        let mut found = Vec::with_capacity(entries.len());
        for entry in entries {
            let mut shown = match entry {
                Entry::Conflict { kind, id, field } => {
                    self.conflict(entry, kind, id, field, &thread)?
                },
                Entry::Unpushed { remote } => json!({"kind": entry.kind(), "remote": remote}),
                Entry::WaitsForCycle { items: ids } | Entry::ParentCycle { items: ids } => {
                    json!({"kind": entry.kind(), "items": items(ids)?})
                },
                Entry::ThreadCycle { threads } => json!({
                    "kind": entry.kind(),
                    "threads": threads.iter().map(|id| thread(id)).collect::<Vec<_>>(),
                }),
                Entry::Contest { thread: id, holders } => {
                    json!({"kind": entry.kind(), "thread": thread(id), "holders": holders})
                },
                Entry::ClockAhead { thread: id, holder } => {
                    json!({"kind": entry.kind(), "thread": thread(id), "holder": holder})
                },
                Entry::Stalled { thread: id, holder, expired_at } => json!({
                    "kind": entry.kind(), "thread": thread(id), "holder": holder,
                    "expired_at": expired_at,
                }),
                Entry::Unthreaded { item, fix } => {
                    json!({"kind": entry.kind(), "item": self.item_row(item)?, "fix": fix})
                },
                Entry::ThreadOnParent { item, thread: held, fix } => json!({
                    "kind": entry.kind(), "item": self.item_row(item)?, "thread": thread(held), "fix": fix,
                }),
                Entry::EmptyThread { thread: id, fix } => {
                    json!({"kind": entry.kind(), "thread": thread(id), "fix": fix})
                },
                Entry::DoneWaiting { item, waits_for }
                | Entry::WaitsOnCanceled { item, waits_for } => json!({
                    "kind": entry.kind(), "item": self.item_row(item)?, "waits_for": self.item_row(waits_for)?,
                }),
                Entry::Dangling { kind, id, field, target, fix } => {
                    let mut shown = json!({
                        "kind": entry.kind(), "record": kind, "id": id, "field": field, "target": target,
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
            };
            shown["key"] = entry.key().into();
            shown["summary"] = entry.summary(&name).into();
            if !matches!(entry, Entry::Conflict { .. }) {
                shown["identity"] = entry.identity().into();
            }
            shown["notify"] = entry.notifies().into();
            found.push(shown);
        }
        Ok(found)
    }

    /// A conflict: the entity and field, the values written apart (a clear among them as `null`), how each is kept
    /// (`keeps`: its command, or why none), and the entity's summary when it is an item or thread.
    fn conflict(
        &self,
        entry: &Entry,
        entity: &str,
        id: &str,
        field: &str,
        thread: &dyn Fn(&str) -> Json,
    ) -> Result<Json, CliError> {
        let by = acyclic_core::model::By { actor: acyclic_core::scope::USER, at: self.now };
        let kept = acyclic_core::needs::keeps(self.state, self.model, (entity, id, field), by)?;
        let values: Vec<&Json> = kept.iter().map(|kept| &kept.value).collect();
        let keeps: Vec<Json> =
            kept.iter().map(|kept| crate::keep::kept_json(self.model, kept)).collect();
        let identity = crate::keep::conflict_identity(self.model, entry, &kept);
        let mut shown = json!({
            "kind": entry.kind(), "record": entity, "id": id,
            "field": acyclic_core::model::shown_field(entity, field), "values": values,
            "keeps": keeps, "identity": identity,
        });
        match entity {
            "item" => shown["item"] = self.item_row(id)?,
            "thread" => shown["thread"] = thread(id),
            _ => {},
        }
        Ok(shown)
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

/// Leaves by stage, as a parent's or a thread's `progress.counts` gives them: `leaves` in all, and each leaf under its
/// stage, so the stages add up to `leaves`. Canceled leaves are counted apart, under `canceled`, and are not leaves.
fn stage_counts(stages: &Stages) -> Json {
    json!({
        "done": stages.done, "leaves": stages.total(),
        "in_progress": stages.in_progress, "ready": stages.ready, "waiting": stages.waiting,
        "stalled": stages.stalled, "parked": stages.parked, "unthreaded": stages.unthreaded,
        "canceled": stages.canceled,
    })
}

/// The thread summary: `{id, title}`; its `progress`, where a thread list or show gives it, counts member leaves. A
/// thread only an item names has no title.
pub(crate) fn thread_row(model: &Model, id: &str) -> Json {
    json!({"id": id, "title": model.threads.get(id).map(|t| &t.title)})
}

/// Every thread's summary, in id order.
pub(crate) fn thread_rows(model: &Model) -> Vec<Json> {
    thread_members(model).into_keys().map(|id| thread_row(model, id)).collect()
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

#[cfg(test)]
mod tests {

    use acyclic_core::model::{self, By, add_item, link, set_parent, set_state};
    use acyclic_core::schema::{kind, repository};
    use acyclic_core::scope::{self, ScopeEdit, USER};
    use acyclic_core::state::{Key, Replica, ReplicaId};

    use super::*;

    const NOW: Millis = 1_000;

    fn by(actor: &str) -> By<'_> {
        By { actor, at: NOW }
    }

    fn key(kind: &str, id: &str) -> Key {
        Key::new(kind, id).unwrap()
    }

    /// Items `goal` > `a` and `b` in thread `t`, `late` an agent's add to it, `c` waiting for `a`, `d` done with
    /// evidence, and a repository `r` on `a`; `a` is claimed.
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
        claim::claim(&mut r, "t", "session/pi", NOW, (60_000, 480 * 60_000), false).unwrap();
        r
    }

    fn member(item: &str, thread: Option<&str>) -> ScopeEdit {
        ScopeEdit::Member { item: item.to_owned(), thread: thread.map(str::to_owned) }
    }

    /// Runs `check` over a view of `replica`.
    fn viewing<T>(replica: &Replica, check: impl FnOnce(&View<'_>) -> T) -> T {
        let model = Model::read(replica.state()).unwrap();
        let readiness = ReadinessEvaluator::new(replica.state(), &model, NOW);
        check(&View { state: replica.state(), model: &model, readiness: &readiness, now: NOW })
    }

    /// The output conventions, for any JSON a command prints: keys in snake case, the adjectives always a boolean,
    /// the times named `*_at`. A `command` is a request body to send back, in the request's own words.
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
                    // A command to send is the API's request body, in the request's own words.
                    if name == "command" {
                        continue;
                    }
                    // What sits under `counts` is a number, whatever it is called.
                    if path.ends_with(".counts") {
                        assert!(inner.is_u64(), "{here}");
                        continue;
                    }
                    if name == "conflicted" && !inner.is_object() {
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
    fn a_canceled_leaf_reads_canceled_for_its_waiter_and_counts_apart_under_its_parent() {
        let mut r = tracker();
        set_state(&mut r, "a", ItemState::Canceled, by(USER)).unwrap();
        viewing(&r, |view| {
            assert_eq!(
                view.item_row("c").unwrap()["readiness"]["waiting"]["a"],
                json!({"why": "canceled"})
            );
            let counts = &view.item_row("goal").unwrap()["progress"]["counts"];
            assert_eq!((&counts["canceled"], &counts["leaves"]), (&json!(1), &json!(1)));
        });
    }

    #[test]
    fn an_item_is_one_row_that_show_extends() {
        let mut r = tracker();
        // A size an older version stored is retired: it stays in the state but no item shows it.
        r.write(&key(kind::ITEM, "a"), "size", USER, NOW, Some(json!("m"))).unwrap();
        viewing(&r, |view| {
            let row = view.item_row("c").unwrap();
            assert_eq!(
                row,
                json!({"id": "c", "title": "C", "state": "open", "parent": null, "thread": null,
                       "readiness": {"readiness": "waiting", "waiting": {"a": {"why": "open"}}}, "stage": "waiting"})
            );
            // A parent's summary adds its leaves counted by stage; a leaf's has none.
            assert_eq!(
                view.item_row("goal").unwrap()["progress"],
                json!({"counts": {"done": 0, "leaves": 2, "in_progress": 2,
                       "ready": 0, "stalled": 0, "waiting": 0, "parked": 0, "unthreaded": 0, "canceled": 0}})
            );
            // And its stage, rolled up from them under the key a leaf's has: in progress while their thread is
            // claimed though none is started.
            assert_eq!(view.item_row("goal").unwrap()["stage"], "in-progress");
            let shown = view.item("a", &view.model.items["a"]).unwrap();
            assert_eq!((shown.get("progress"), shown.get("size")), (None, None));
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
            assert_eq!(shown["done_at"], Json::Null);
            let done = view.item("d", &view.model.items["d"]).unwrap();
            // The commit by its repository's owner/name, which a rename changes.
            assert_eq!(done["evidence"], json!({"commit": "me/code@abc1234", "text": "shipped"}));
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
            let rows = view
                .ready_rows(&[ReadyThread { thread: "t".into(), items, stalled: false }])
                .unwrap();
            let summary = thread_row(view.model, "t");
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
            let members: Vec<&Json> =
                shown["members"].as_array().unwrap().iter().map(|m| &m["id"]).collect();
            assert_eq!(members, ["a", "b", "late"]);
            assert_eq!(shown["members"][2]["conflicted"], false);
            let row = thread_row(view.model, "t");
            for (name, value) in row.as_object().unwrap() {
                assert_eq!(&shown[name], value, "{name}");
            }
            follows_the_conventions(&shown, "thread");
        });
    }

    #[test]
    fn a_thread_stored_on_a_parent_is_in_no_row_member_list_or_thread() {
        let mut r = tracker();
        // `goal` has children, so a thread stored on it, as a sync can leave one, is ignored.
        r.write(&model::item_key("goal").unwrap(), "thread", "session/mac", NOW, Some(json!("t")))
            .unwrap();
        viewing(&r, |view| {
            assert_eq!(view.item_row("goal").unwrap()["thread"], json!(null));
            let listed = thread_members(view.model);
            assert_eq!(listed["t"].iter().filter(|member| member.item == "goal").count(), 0);
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
            assert_eq!(parent["record"], "item");
            assert_eq!(parent["item"]["id"], "b");
            assert_eq!(parent["values"], json!([null, "a"]));
            // Each value with the command that keeps it.
            let runs: Vec<&Json> =
                parent["keeps"].as_array().unwrap().iter().map(|k| &k["run"]).collect();
            assert_eq!(
                runs,
                [
                    &json!("acyclic item edit b --no-parent"),
                    &json!("acyclic item edit b --parent a")
                ]
            );
            // A thread cycle embeds each thread's summary.
            let cycle = of("thread-cycle")[0];
            assert_eq!(cycle["threads"], json!(["t", "u"].map(|id| thread_row(view.model, id))));
            assert_eq!(cycle["notify"], true);
            for (entry, shown) in entries.iter().zip(&needs) {
                assert!(shown["kind"].is_string(), "{shown}");
                // Each carries the core's key, which the app and pushes use as given.
                assert_eq!(shown["key"], json!(entry.key()), "{shown}");
                // And the core's line, which is display only: the identity never carries it.
                assert!(shown["summary"].as_str().is_some_and(|line| !line.is_empty()), "{shown}");
                assert!(
                    !shown["identity"]
                        .as_str()
                        .unwrap()
                        .contains(shown["summary"].as_str().unwrap()),
                    "{shown}"
                );
                if matches!(entry, Entry::Conflict { .. }) {
                    let identity: Json =
                        serde_json::from_str(shown["identity"].as_str().unwrap()).unwrap();
                    assert_eq!(identity["entry"], json!(entry), "{shown}");
                    let choices = identity["choices"].as_array().unwrap();
                    let displayed = shown["keeps"].as_array().unwrap();
                    assert_eq!(choices.len(), displayed.len(), "{shown}");
                    for (choice, keep) in choices.iter().zip(displayed) {
                        assert_eq!(choice["value"], keep["value"], "{shown}");
                        assert_eq!(choice["command"], keep["command"], "{shown}");
                    }
                } else {
                    assert_eq!(shown["identity"], json!(entry.identity()), "{shown}");
                }
                follows_the_conventions(shown, "needs");
            }
        });
    }
}
