use serde_json::json;

use super::*;
use crate::model::{ItemState, Level, add_item, set_state};
use crate::needs::{Entry, needs_you};
use crate::ready::{Ancestry, Readiness};
use crate::state::{ReplicaId, State};

const NOW: Millis = 10_000;

fn user(at: Millis) -> By<'static> {
    By { actor: USER, at }
}

fn replica(name: &str) -> Replica {
    Replica::open(ReplicaId::new(name).unwrap(), State::new())
}

fn key(id: &str) -> Key {
    Key::new(kind::THREAD, id).unwrap()
}

fn put(r: &mut Replica, key: &Key, field: &str, value: Json) {
    r.write(key, field, USER, 1, Some(value)).unwrap();
}

/// A replica on `name` holding thread `id`, titled and approved at the user's level, with members `<name>-a` (done)
/// and `<name>-b`.
fn with_thread(name: &str, id: &str) -> Replica {
    let mut r = replica(name);
    let t = key(id);
    put(&mut r, &t, thread::TITLE, json!("Beds"));
    put(&mut r, &t, thread::BRIEF, json!("Dig them"));
    put(&mut r, &t, thread::APPROVED, json!(true));
    let members = [format!("{name}-a"), format!("{name}-b")];
    put(&mut r, &t, thread::APPROVED_MEMBERS, json!(members));
    put(&mut r, &t, thread::AUTONOMY, json!(Level::Everything));
    put(&mut r, &t, &format!("{}code", set::REPOSITORY), json!(true));
    for item in &members {
        add_item(&mut r, item, item, user(1)).unwrap();
        model::put(&mut r, &model::item_key(item).unwrap(), item::THREAD, Some(&id), user(1))
            .unwrap();
    }
    set_state(&mut r, &members[0], ItemState::Done, user(1)).unwrap();
    r
}

/// Gives `r` an older thread whose id ends in 32 too, which `thread-pi-32` clashes with.
fn partnered(mut r: Replica) -> Replica {
    put(&mut r, &key("thread-32"), thread::TITLE, json!("Partner"));
    r
}

fn why(result: Result<Renumbered, RenumberError>) -> Vec<String> {
    match result {
        Err(RenumberError::InUse { reasons, .. }) => reasons
            .into_iter()
            .map(|reason| serde_json::to_value(reason).unwrap()["why"].as_str().unwrap().to_owned())
            .collect(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_fresh_thread_moves_with_its_fields_and_members_and_its_old_id_is_retired() {
    let mut r = partnered(with_thread("mac", "thread-pi-32"));
    let done = renumber(&mut r, "thread-pi-32", user(2)).unwrap();
    assert_eq!(
        done,
        Renumbered {
            thread: "thread-mac-33".into(),
            from: "thread-pi-32".into(),
            moved: vec!["mac-a".into(), "mac-b".into()]
        }
    );
    let model = Model::read(r.state()).unwrap();
    assert!(!model.threads.contains_key("thread-pi-32"));
    let moved = &model.threads["thread-mac-33"];
    assert_eq!(
        (moved.title.one().map(String::as_str), moved.is_approved(), moved.autonomy.one()),
        (Some("Beds"), true, Some(&Level::Everything))
    );
    assert_eq!(moved.approved_members.one(), Some(&vec!["mac-a".to_owned(), "mac-b".to_owned()]));
    assert!(moved.repositories.contains("code"));
    for item in ["mac-a", "mac-b"] {
        assert_eq!(model.items[item].thread.one().map(String::as_str), Some("thread-mac-33"));
    }
    // The old id is never given again, and adding it back is refused.
    assert_eq!(ids::next(r.state(), kind::THREAD, r.id()), "thread-mac-34");
    assert!(model::is_deleted(r.state(), &key("thread-pi-32")));
    assert!(matches!(renumber(&mut r, "thread-pi-32", user(3)), Err(RenumberError::NoThread(_))));
}

#[test]
fn only_the_user_renumbers_and_only_a_fresh_thread_that_clashes() {
    let mut r = with_thread("mac", "t");
    let agent = By { actor: "session/mac", at: 2 };
    assert!(matches!(renumber(&mut r, "t", agent), Err(RenumberError::NotTheUser(_))));
    assert!(matches!(renumber(&mut r, "nope", user(2)), Err(RenumberError::NoThread(_))));
    let question = Key::new(kind::QUESTION, "q1").unwrap();
    put(&mut r, &question, question::THREAD, json!("t"));
    claim::claim(&mut r, "t", "session/s", 2, NOW, NOW).unwrap();
    // An agent's membership edit into the approved thread waits as a proposal.
    add_item(&mut r, "c", "c", user(1)).unwrap();
    let edit = ScopeEdit::Member { item: "c".into(), thread: Some("t".into()) };
    scope::edit(&mut r, &edit, By { actor: "session/s", at: 2 }).unwrap();
    // Fields it would carry in conflict: two replicas' briefs, and the members each approval saw.
    let mut other = with_thread("pi", "t");
    put(&mut other, &key("t"), thread::BRIEF, json!("Rake them"));
    r.join(other.state()).unwrap();
    let before = r.changes();
    assert_eq!(
        why(renumber(&mut r, "t", user(3))),
        ["no-clash", "question", "claimed", "proposal", "conflict", "conflict"]
    );
    assert_eq!(r.changes(), before, "a refusal writes nothing");
}

fn entries(r: &Replica) -> Vec<Entry> {
    let model = Model::read(r.state()).unwrap();
    needs_you(r.state(), &model, &Readiness::new(r.state(), &model, NOW, &Always), NOW).unwrap()
}

fn clash_entries(r: &Replica) -> Vec<Entry> {
    entries(r).into_iter().filter(|entry| matches!(entry, Entry::NumberClash { .. })).collect()
}

#[test]
fn threads_ending_in_one_number_clash_until_a_fresh_one_is_renumbered() {
    let mut mac = with_thread("mac", "thread-mac-32");
    let pi = with_thread("pi", "thread-pi-32");
    put(&mut mac, &key("thread-27"), thread::TITLE, json!("Older"));
    put(&mut mac, &key("thread-x-27"), thread::TITLE, json!("Imported"));
    mac.join(pi.state()).unwrap();
    let found = clashes(mac.state(), &Model::read(mac.state()).unwrap(), NOW).unwrap();
    assert_eq!(
        found.iter().map(|clash| (clash.number, clash.threads.len())).collect::<Vec<_>>(),
        [(27, 2), (32, 2)]
    );
    assert_eq!(clash_entries(&mac).len(), 2);
    // A clash nothing can resolve stays out of needs, though `clashes` still reports it.
    for thread in ["thread-27", "thread-x-27"] {
        put(&mut mac, &Key::new(kind::QUESTION, thread).unwrap(), question::THREAD, json!(thread));
    }
    let entries = clash_entries(&mac);
    assert!(
        matches!(&entries[..], [Entry::NumberClash { number: 32, fix, .. }] if fix == "acyclic thread renumber thread-mac-32 --as-user"),
        "{entries:?}"
    );
    renumber(&mut mac, "thread-pi-32", user(2)).unwrap();
    assert_eq!(clash_entries(&mac), []);
}

#[test]
fn a_member_added_under_the_old_id_meanwhile_dangles_rather_than_bringing_it_back() {
    let mut mac = partnered(with_thread("mac", "thread-pi-32"));
    let mut pi = with_thread("pi", "thread-pi-32");
    renumber(&mut mac, "thread-pi-32", user(2)).unwrap();
    add_item(&mut pi, "late", "late", user(3)).unwrap();
    model::put(
        &mut pi,
        &model::item_key("late").unwrap(),
        item::THREAD,
        Some(&"thread-pi-32"),
        user(3),
    )
    .unwrap();
    mac.join(pi.state()).unwrap();
    assert!(!Model::read(mac.state()).unwrap().threads.contains_key("thread-pi-32"));
    let dangling = entries(&mac).into_iter().any(|entry| matches!(entry, Entry::Dangling { ref id, ref target, .. } if id == "late" && target == "thread-pi-32"));
    assert!(dangling);
}

struct Always;
impl Ancestry for Always {
    fn merged(&self, _: &crate::model::Repository, _: &str) -> Result<bool, String> {
        Ok(true)
    }
}
