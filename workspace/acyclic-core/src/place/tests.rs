use serde_json::json;

use super::*;
use crate::model::{ItemState, add_item, item_key, link, set_parent, set_state};
use crate::schema::{item as field, kind, thread};
use crate::state::{Key, ReplicaId, State};

const USER: By<'static> = By { actor: "actor/user", at: 1, seen: None };

fn replica_named(name: &str) -> Replica {
    Replica::open(ReplicaId::new(name).unwrap(), State::new())
}

fn write(r: &mut Replica, key: &Key, name: &str, value: serde_json::Value) {
    r.write(key, name, USER.actor, USER.at, Some(value)).unwrap();
}

fn add(r: &mut Replica, id: &str, parent: Option<&str>) {
    add_item(r, id, &format!("Title {id}"), USER).unwrap();
    if let Some(parent) = parent {
        set_parent(r, id, Some(parent), USER).unwrap();
    }
}

fn set_thread(r: &mut Replica, id: &str, thread: &str) {
    write(r, &item_key(id).unwrap(), field::THREAD, json!(thread));
}

/// Top `p`, group `g1` under it, and `g1`'s leaves `g1-00` to `g1-<n-1>`.
fn group(n: usize) -> Replica {
    let mut r = replica_named("mac");
    add(&mut r, "p", None);
    add(&mut r, "g1", Some("p"));
    for i in 0..n {
        add(&mut r, &format!("g1-{i:02}"), Some("g1"));
    }
    r
}

fn model(r: &Replica) -> Model {
    Model::read(r.state()).unwrap()
}

fn place_one(r: &mut Replica, item: &str, actor: &'static str) -> Vec<Inherited> {
    place(r, &[item.to_owned()], By { actor, at: 9, seen: None }).unwrap()
}

#[test]
fn new_work_joins_its_nearest_threaded_ancestor_as_its_actor_may() {
    let user = By { actor: USER.actor, at: 1, seen: None };
    let mut r = group(1);
    for (thread, approved, pm) in [("ok", true, false), ("loose", false, false), ("pm", true, true)]
    {
        let key = Key::new(kind::THREAD, thread).unwrap();
        write(&mut r, &key, thread::APPROVED, json!(approved));
        write(&mut r, &key, thread::PM_FLAG, json!(pm));
    }
    crate::model::start_run(&mut r, "boss", "pm", user).unwrap();
    set_thread(&mut r, "g1", "ok");
    // The user and a project-management run join approved scope; any other agent proposes.
    for (leaf, actor, proposed) in
        [("u", "actor/user", false), ("m", "run/boss", false), ("a", "session/mac", true)]
    {
        add(&mut r, leaf, Some("g1-00"));
        let placed = place_one(&mut r, leaf, actor);
        let [joined] = placed.as_slice() else { panic!("{actor}: {placed:?}") };
        assert_eq!(
            (joined.thread.as_str(), joined.proposal.is_some()),
            ("ok", proposed),
            "{actor}"
        );
    }
    // Unapproved scope takes anyone's work; a thread an item already has is never changed.
    set_thread(&mut r, "g1", "loose");
    add(&mut r, "b", Some("g1"));
    assert_eq!(place_one(&mut r, "b", "session/mac")[0].proposal, None);
    add(&mut r, "c", Some("g1-00"));
    set_thread(&mut r, "c", "own");
    assert_eq!(place_one(&mut r, "c", "session/mac"), Vec::new());
    assert_eq!(model(&r).thread_of("c"), Some("own"));
    // Work with no thread above it, or around it, stays as it is.
    add(&mut r, "loner", None);
    assert_eq!(place_one(&mut r, "loner", USER.actor), Vec::new());
    // Under a parent in conflict there is no single line to inherit along.
    let mut other = replica_named("pi");
    add(&mut r, "torn", None);
    other.join(r.state()).unwrap();
    set_parent(&mut r, "torn", Some("g1-00"), By { actor: USER.actor, at: 20, seen: None })
        .unwrap();
    set_parent(&mut other, "torn", Some("p"), By { actor: USER.actor, at: 20, seen: None })
        .unwrap();
    r.join(other.state()).unwrap();
    assert_eq!(inherited_thread(&model(&r), "torn"), None);
}

#[test]
fn a_groups_new_work_joins_the_thread_its_other_work_is_in() {
    let mut r = replica_named("mac");
    add(&mut r, "move", None);
    add(&mut r, "boxes", Some("move"));
    assert_eq!(place_one(&mut r, "boxes", USER.actor), Vec::new(), "nothing to follow yet");
    set_thread(&mut r, "boxes", "t");
    for kid in ["pack", "van"] {
        add(&mut r, kid, Some("move"));
        let placed = place(&mut r, &[kid.to_owned()], USER).unwrap();
        assert_eq!(placed[0].thread, "t");
    }
    assert_eq!(model(&r).members("t").len(), 3);
}

#[test]
fn siblings_pass_their_thread_down_only_when_they_agree() {
    let mut r = group(2);
    let shared = |r: &Replica| passed_down(&model(r), "g1").map(str::to_owned);
    for leaf in ["g1-00", "g1-01"] {
        set_thread(&mut r, leaf, "t");
    }
    set_thread(&mut r, "p", "up");
    assert_eq!(shared(&r).as_deref(), Some("t"), "nearer than the grandparent's");
    set_thread(&mut r, "g1-01", "u");
    assert_eq!(shared(&r).as_deref(), Some("up"), "siblings that differ fall through");
    set_state(&mut r, "g1-01", ItemState::Done, USER).unwrap();
    assert_eq!(shared(&r).as_deref(), Some("t"), "finished work pulls nothing in");
    // Siblings in conflict fall through; a parent in conflict stops the walk, however its children agree.
    let mut other = replica_named("pi");
    other.join(r.state()).unwrap();
    let key = item_key("g1-00").unwrap();
    other.write(&key, field::THREAD, USER.actor, 5, Some(json!("v"))).unwrap();
    r.write(&key, field::THREAD, USER.actor, 5, Some(json!("t"))).unwrap();
    r.join(other.state()).unwrap();
    assert_eq!(shared(&r).as_deref(), Some("up"));
    set_thread(&mut r, "g1-00", "t");
    let key = item_key("g1").unwrap();
    other.write(&key, field::THREAD, USER.actor, 6, Some(json!("v"))).unwrap();
    r.write(&key, field::THREAD, USER.actor, 6, Some(json!("w"))).unwrap();
    r.join(other.state()).unwrap();
    assert_eq!(shared(&r), None);
}

#[test]
fn an_agents_sibling_join_into_approved_scope_is_a_proposal() {
    let mut r = group(1);
    let ok = Key::new(kind::THREAD, "ok").unwrap();
    write(&mut r, &ok, thread::APPROVED, json!(true));
    set_thread(&mut r, "g1-00", "ok");
    add(&mut r, "late", Some("g1"));
    let agent = By { actor: "session/mac", at: 9, seen: None };
    let placed = place(&mut r, &["late".to_owned()], agent).unwrap();
    let [joined] = placed.as_slice() else { panic!("{placed:?}") };
    assert_eq!((joined.thread.as_str(), joined.proposal.is_some()), ("ok", true));
}

#[test]
fn threads_that_wait_on_each_other_are_named_and_a_link_on_a_group_counts_for_its_work() {
    let mut r = group(0);
    for (leaf, thread) in [("x", "t"), ("x2", "t"), ("y", "u"), ("y2", "u")] {
        add(&mut r, leaf, Some("g1"));
        set_thread(&mut r, leaf, thread);
    }
    link(&mut r, "x", "y", LinkKind::Blocks, USER).unwrap();
    assert_eq!(thread_cycles(&model(&r)), Vec::<BTreeSet<String>>::new());
    add(&mut r, "box", None);
    set_parent(&mut r, "x2", Some("box"), USER).unwrap();
    link(&mut r, "y2", "box", LinkKind::Blocks, USER).unwrap();
    let cycle = BTreeSet::from(["t".to_owned(), "u".to_owned()]);
    assert_eq!(thread_cycles(&model(&r)), vec![cycle]);
    set_state(&mut r, "x2", ItemState::Done, USER).unwrap();
    assert_eq!(
        thread_cycles(&model(&r)),
        Vec::<BTreeSet<String>>::new(),
        "done work waits on nothing"
    );
}

#[test]
fn threads_wait_only_on_open_leaves_as_the_waves_count_them() {
    let mut r = group(0);
    for (leaf, thread) in [("x", "t"), ("y", "u")] {
        add(&mut r, leaf, Some("g1"));
        set_thread(&mut r, leaf, thread);
    }
    link(&mut r, "x", "y", LinkKind::Blocks, USER).unwrap();
    add(&mut r, "old", None);
    add(&mut r, "box", Some("old"));
    add(&mut r, "z", Some("box"));
    set_thread(&mut r, "z", "u");
    link(&mut r, "y", "box", LinkKind::Blocks, USER).unwrap();
    let cycle = vec![BTreeSet::from(["t".to_owned(), "u".to_owned()])];
    assert_eq!(
        thread_cycles(&model(&r)),
        Vec::<BTreeSet<String>>::new(),
        "z is in u, the thread waiting"
    );
    set_thread(&mut r, "z", "t");
    assert_eq!(thread_cycles(&model(&r)), cycle);
    write(&mut r, &item_key("old").unwrap(), field::ARCHIVED, json!(true));
    assert_eq!(
        thread_cycles(&model(&r)),
        Vec::<BTreeSet<String>>::new(),
        "work under an archived ancestor of the linked group holds nothing back"
    );
    write(&mut r, &item_key("old").unwrap(), field::ARCHIVED, json!(false));
    write(&mut r, &item_key("z").unwrap(), field::ARCHIVED, json!(true));
    assert_eq!(
        thread_cycles(&model(&r)),
        Vec::<BTreeSet<String>>::new(),
        "nor does archived work itself"
    );
    write(&mut r, &item_key("z").unwrap(), field::ARCHIVED, json!(false));
    set_thread(&mut r, "z", "u");
    set_thread(&mut r, "box", "t");
    assert_eq!(
        thread_cycles(&model(&r)),
        Vec::<BTreeSet<String>>::new(),
        "a group's own thread holds no work"
    );
}

#[test]
fn concurrent_threads_agree_when_equal_and_conflict_when_not() {
    let mut a = group(1);
    let mut b = replica_named("pi");
    b.join(a.state()).unwrap();
    let key = item_key("g1-00").unwrap();
    a.write(&key, field::THREAD, USER.actor, 5, Some(json!("t"))).unwrap();
    b.write(&key, field::THREAD, USER.actor, 5, Some(json!("t"))).unwrap();
    a.join(b.state()).unwrap();
    assert_eq!(model(&a).thread_of("g1-00"), Some("t"), "inheritance sees one thread");
    b.write(&key, field::THREAD, USER.actor, 6, Some(json!("u"))).unwrap();
    a.write(&key, field::THREAD, USER.actor, 6, Some(json!("v"))).unwrap();
    a.join(b.state()).unwrap();
    assert!(model(&a).items["g1-00"].thread.is_conflict(), "and passes none down");
}
