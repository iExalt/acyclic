use serde_json::json;

use super::*;
use crate::state::ReplicaId;

fn replica(name: &str) -> Replica {
    Replica::open(ReplicaId::new(name).unwrap(), State::new())
}

fn by(at: Millis) -> By<'static> {
    By { actor: "actor/user", at }
}

fn items(replica: &mut Replica, ids: &[&str]) {
    for id in ids {
        add_item(replica, id, id, by(1)).unwrap();
    }
}

fn model(replica: &Replica) -> Model {
    Model::read(&replica.state).unwrap()
}

/// Two replicas that share the given items, so each can then write concurrently.
fn pair(ids: &[&str]) -> (Replica, Replica) {
    let mut a = replica("a");
    items(&mut a, ids);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    (a, b)
}

#[test]
fn roles_follow_children_gates_and_the_top_level() {
    let mut r = replica("a");
    items(&mut r, &["top", "gated", "plain", "leaf", "leaf2", "gated-top", "under-gated-top"]);
    set_parent(&mut r, "gated", Some("top"), by(2)).unwrap();
    set_parent(&mut r, "plain", Some("top"), by(2)).unwrap();
    set_parent(&mut r, "leaf", Some("gated"), by(2)).unwrap();
    set_parent(&mut r, "leaf2", Some("plain"), by(2)).unwrap();
    set_parent(&mut r, "under-gated-top", Some("gated-top"), by(2)).unwrap();
    for id in ["gated", "gated-top"] {
        set(&mut r, &item_key(id).unwrap(), field::GATE, Some(&"ships"), by(3)).unwrap();
    }
    let m = model(&r);
    assert_eq!(m.role("top"), Role::Project);
    assert_eq!(m.role("gated"), Role::Goal);
    assert_eq!(m.role("plain"), Role::Group);
    assert_eq!(m.role("leaf"), Role::Item);
    // A top-level parent is a project even with a gate.
    assert_eq!(m.role("gated-top"), Role::Project);
    // An unplaced leaf is an item, not a project.
    let mut r = replica("b");
    items(&mut r, &["alone"]);
    assert_eq!(model(&r).role("alone"), Role::Item);
}

#[test]
fn an_item_never_goes_under_itself_or_a_descendant() {
    let mut r = replica("a");
    items(&mut r, &["a", "b", "c"]);
    set_parent(&mut r, "b", Some("a"), by(2)).unwrap();
    set_parent(&mut r, "c", Some("b"), by(2)).unwrap();
    assert_eq!(set_parent(&mut r, "a", Some("a"), by(3)), Err(WriteError::ParentCycle("a".into())));
    assert_eq!(set_parent(&mut r, "a", Some("c"), by(3)), Err(WriteError::ParentCycle("a".into())));
    assert_eq!(
        set_parent(&mut r, "a", Some("missing"), by(3)),
        Err(WriteError::NoItem("missing".into()))
    );
    set_parent(&mut r, "c", None, by(3)).unwrap();
    assert_eq!(model(&r).ancestors("c"), Vec::<&str>::new());
}

#[test]
fn a_parent_cycle_made_by_a_join_is_reported_and_every_walk_ends() {
    let (mut a, mut b) = pair(&["x", "y"]);
    set_parent(&mut a, "x", Some("y"), by(2)).unwrap();
    set_parent(&mut b, "y", Some("x"), by(2)).unwrap();
    a.join(&b.state).unwrap();
    let m = model(&a);
    assert_eq!(m.parent_cycles(), vec![vec!["x".to_owned(), "y".to_owned()]]);
    assert_eq!(m.ancestors("x"), vec!["y"]);
    assert_eq!(m.subtree("x").len(), 2);
    assert_eq!(m.role("x"), Role::Group);
    assert!(!m.is_archived("x"));
    put(&mut a, &item_key("y").unwrap(), field::ARCHIVED, Some(&true), by(3)).unwrap();
    assert!(model(&a).is_archived("x"), "inherited through the cycle, and the walk ends");
}

#[test]
fn a_conflicted_parent_counts_as_each_of_its_values() {
    let (mut a, mut b) = pair(&["p1", "p2", "child"]);
    set_parent(&mut a, "child", Some("p1"), by(2)).unwrap();
    set_parent(&mut b, "child", Some("p2"), by(2)).unwrap();
    a.join(&b.state).unwrap();
    let m = model(&a);
    assert!(m.items["child"].parent.is_conflict());
    let mut ancestors = m.ancestors("child");
    ancestors.sort_unstable();
    assert_eq!(ancestors, vec!["p1", "p2"]);
    assert_eq!((m.children("p1"), m.children("p2")), (vec!["child"], vec!["child"]));
}

#[test]
fn a_clear_racing_a_set_conflicts_while_clears_agree_and_an_old_empty_clear_reads_alike() {
    let notes = |r: &Replica| model(r).items["x"].notes.clone();
    let key = item_key("x").unwrap();
    let (mut a, mut b) = pair(&["x"]);
    set(&mut a, &key, field::NOTES, Some(&"first"), by(2)).unwrap();
    b.join(&a.state).unwrap();
    set::<String>(&mut a, &key, field::NOTES, None, by(3)).unwrap();
    set(&mut b, &key, field::NOTES, Some(&"second"), by(3)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(notes(&a), Value::Conflict(vec![None, Some("second".to_owned())]));
    assert_eq!(serde_json::to_value(notes(&a)).unwrap(), json!({"conflict": [null, "second"]}));
    // A clear after seeing both settles it; so do two clears that race.
    set::<String>(&mut b, &key, field::NOTES, None, by(4)).unwrap();
    b.join(&a.state).unwrap();
    set::<String>(&mut a, &key, field::NOTES, None, by(4)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(notes(&a), Value::None);
    // A clear written before clears were values left an empty register, which reads the same.
    let mut old = replica("old");
    items(&mut old, &["x"]);
    old.write(&key, field::NOTES, "actor/user", 2, None).unwrap();
    assert_eq!(notes(&old), Value::None);
    // Set after a clear, in order, is just the set.
    set(&mut a, &key, field::NOTES, Some(&"third"), by(5)).unwrap();
    assert_eq!(notes(&a), Value::One("third".to_owned()));
}

#[test]
fn ordering_links_never_close_a_cycle_locally() {
    let mut r = replica("a");
    items(&mut r, &["a", "b", "c"]);
    link(&mut r, "a", "b", LinkKind::Blocks, by(2)).unwrap();
    link(&mut r, "b", "c", LinkKind::After, by(2)).unwrap();
    // Hard and soft links count together.
    let err = link(&mut r, "c", "a", LinkKind::Blocks, by(3)).unwrap_err();
    assert_eq!(err, WriteError::LinkCycle(vec!["a".into(), "b".into(), "c".into()]));
    for kind in [LinkKind::Blocks, LinkKind::After, LinkKind::Relates] {
        assert_eq!(link(&mut r, "a", "a", kind, by(3)), Err(WriteError::SelfLink("a".into())));
    }
    // Context links don't order anything.
    link(&mut r, "c", "a", LinkKind::Relates, by(3)).unwrap();
    assert_eq!(model(&r).link_cycles(), Vec::<Vec<String>>::new());
}

#[test]
fn a_link_cycle_made_by_a_join_is_derived() {
    let (mut a, mut b) = pair(&["x", "y"]);
    link(&mut a, "x", "y", LinkKind::Blocks, by(2)).unwrap();
    link(&mut b, "y", "x", LinkKind::After, by(2)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(model(&a).link_cycles(), vec![vec!["x".to_owned(), "y".to_owned()]]);
}

#[test]
fn one_link_added_twice_is_one_link_and_a_re_add_beats_a_removal() {
    let (mut a, mut b) = pair(&["x", "y"]);
    link(&mut a, "x", "y", LinkKind::Blocks, by(2)).unwrap();
    link(&mut b, "x", "y", LinkKind::Blocks, by(2)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(model(&a).links.len(), 1);
    b.join(&a.state).unwrap();
    unlink(&mut a, "x", "y", LinkKind::Blocks, by(3)).unwrap();
    assert_eq!(model(&a).links, Vec::new());
    link(&mut b, "x", "y", LinkKind::Blocks, by(3)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(model(&a).links.len(), 1, "the concurrent re-add wins");
}

#[test]
fn link_ids_are_unambiguous() {
    let one = link_key("a-b", "c", LinkKind::Blocks).unwrap();
    let two = link_key("a", "b-c", LinkKind::Blocks).unwrap();
    assert_ne!(one, two);
}

#[test]
fn set_elements_merge_and_a_concurrent_add_beats_a_removal() {
    let (mut a, mut b) = pair(&["x"]);
    let key = item_key("x").unwrap();
    set_element(&mut a, &key, SetField::Area, "ui", true, by(2)).unwrap();
    set_element(&mut b, &key, SetField::Area, "core", true, by(2)).unwrap();
    a.join(&b.state).unwrap();
    b.join(&a.state).unwrap();
    assert_eq!(model(&a).items["x"].areas, BTreeSet::from(["core".into(), "ui".into()]));
    set_element(&mut a, &key, SetField::Area, "ui", false, by(3)).unwrap();
    set_element(&mut b, &key, SetField::Area, "ui", true, by(3)).unwrap();
    a.join(&b.state).unwrap();
    assert!(model(&a).items["x"].areas.contains("ui"));
}

#[test]
fn names_follow_the_id_rule() {
    let mut r = replica("a");
    items(&mut r, &["x"]);
    let key = item_key("x").unwrap();
    for bad in ["a/b", "Engine", "", ".hidden"] {
        assert_eq!(
            set_element(&mut r, &key, SetField::Resource, bad, true, by(2)),
            Err(WriteError::InvalidName(bad.to_owned()))
        );
    }
    let long = "a".repeat(MAX_ID_LEN + 1);
    assert!(matches!(add_item(&mut r, &long, "t", by(2)), Err(WriteError::InvalidName(_))));
    add_item(&mut r, &"a".repeat(MAX_ID_LEN), "t", by(2)).unwrap();
}

#[test]
fn a_conflicted_thread_puts_an_item_in_neither_and_repository_sets_are_unions() {
    let (mut a, mut b) = pair(&["x", "y"]);
    let thread_key = |id| Key::new(kind::THREAD, id).unwrap();
    set(&mut a, &thread_key("t1"), thread::TITLE, Some(&"one"), by(2)).unwrap();
    put_element(&mut a, &thread_key("t1"), set::REPOSITORY, "docs", true, by(2)).unwrap();
    for (id, repo) in [("x", "code"), ("y", "infra")] {
        put_element(&mut a, &item_key(id).unwrap(), set::REPOSITORY, repo, true, by(2)).unwrap();
    }
    put(&mut a, &item_key("x").unwrap(), field::THREAD, Some(&"t1"), by(2)).unwrap();
    put(&mut a, &item_key("y").unwrap(), field::THREAD, Some(&"t1"), by(2)).unwrap();
    put(&mut b, &item_key("y").unwrap(), field::THREAD, Some(&"t2"), by(2)).unwrap();
    a.join(&b.state).unwrap();
    let m = model(&a);
    assert_eq!(m.members("t1"), vec!["x"]);
    assert_eq!(m.members("t2"), Vec::<&str>::new());
    assert_eq!(m.thread_of("y"), None);
    assert_eq!(m.repository_set("t1"), BTreeSet::from(["code".into(), "docs".into()]));
}

#[test]
fn a_threads_heads_are_its_open_leaves_that_no_link_orders_after_another_member() {
    let mut r = replica("a");
    items(&mut r, &["a", "b", "c", "d", "p", "x"]);
    for id in ["a", "b", "c", "d"] {
        put(&mut r, &item_key(id).unwrap(), field::THREAD, Some(&"t"), by(2)).unwrap();
    }
    let heads =
        |r: &Replica| model(r).heads("t").into_iter().map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(heads(&r), ["a", "b", "c", "d"], "membership alone orders nothing");
    link(&mut r, "b", "a", LinkKind::Blocks, by(3)).unwrap();
    link(&mut r, "a", "d", LinkKind::Relates, by(3)).unwrap();
    // Through an item outside the thread, a soft link orders too.
    link(&mut r, "c", "x", LinkKind::After, by(3)).unwrap();
    link(&mut r, "x", "a", LinkKind::Blocks, by(3)).unwrap();
    // A parent's links order what is under it.
    set_parent(&mut r, "d", Some("p"), by(3)).unwrap();
    link(&mut r, "p", "a", LinkKind::Blocks, by(3)).unwrap();
    assert_eq!(heads(&r), ["a"]);
    set_state(&mut r, "a", ItemState::Done, by(4)).unwrap();
    assert_eq!(heads(&r), ["b", "c", "d"], "a done member orders nothing");
    put(&mut r, &item_key("c").unwrap(), field::ARCHIVED, Some(&true), by(5)).unwrap();
    // A parent in the thread is never a head, and following it follows what is under it.
    put(&mut r, &item_key("p").unwrap(), field::THREAD, Some(&"t"), by(5)).unwrap();
    link(&mut r, "b", "p", LinkKind::After, by(5)).unwrap();
    assert_eq!(heads(&r), ["d"]);
}

#[test]
fn done_at_is_the_first_done_and_re_marking_done_writes_nothing() {
    let (mut a, mut b) = pair(&["x"]);
    set_state(&mut a, "x", ItemState::Done, by(10)).unwrap();
    set_state(&mut b, "x", ItemState::Done, by(20)).unwrap();
    a.join(&b.state).unwrap();
    let m = model(&a);
    assert_eq!(
        (m.items["x"].state.clone(), m.items["x"].done_at),
        (Value::One(ItemState::Done), Some(10))
    );
    let before = a.state.clone();
    set_state(&mut a, "x", ItemState::Done, by(30)).unwrap();
    assert_eq!(a.state, before);
    set_state(&mut a, "x", ItemState::InProgress, by(40)).unwrap();
    assert_eq!(model(&a).items["x"].done_at, None);
}

#[test]
fn a_thread_runs_one_run_at_a_time() {
    let (mut a, mut b) = pair(&[]);
    start_run(&mut a, "r1", "t1", by(1)).unwrap();
    assert_eq!(
        start_run(&mut a, "r2", "t1", by(2)),
        Err(WriteError::RunActive { thread: "t1".into(), run: "r1".into() })
    );
    start_run(&mut b, "r3", "t1", by(1)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(
        model(&a).run_clashes(),
        BTreeMap::from([("t1".into(), vec!["r1".into(), "r3".into()])])
    );
    set_run_state(&mut a, "r1", RunState::Done, by(3)).unwrap();
    assert!(model(&a).run_clashes().is_empty());
}

#[test]
fn equal_concurrent_values_read_as_one_and_unreadable_ones_fail() {
    let (mut a, mut b) = pair(&["x"]);
    let key = item_key("x").unwrap();
    set(&mut a, &key, field::SIZE, Some(&Size::M), by(2)).unwrap();
    set(&mut b, &key, field::SIZE, Some(&Size::M), by(2)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(model(&a).items["x"].size, Value::One(Size::M));
    a.write(&key, field::STATE, "actor/user", 3, Some(json!("from-the-future"))).unwrap();
    assert!(matches!(Model::read(&a.state), Err(Error::Format(_))));
}

#[test]
fn fields_with_invariants_need_their_checked_writes() {
    let mut r = replica("a");
    items(&mut r, &["x", "y"]);
    let x = item_key("x").unwrap();
    for name in [field::PARENT, field::STATE] {
        assert!(matches!(
            set(&mut r, &x, name, Some(&"y"), by(2)),
            Err(WriteError::Checked { .. })
        ));
    }
    let link = link_key("x", "y", LinkKind::Blocks).unwrap();
    assert!(matches!(
        set(&mut r, &link, link::FROM, Some(&"x"), by(2)),
        Err(WriteError::Checked { .. })
    ));
    let run = Key::new(kind::RUN, "r1").unwrap();
    assert!(matches!(
        set(&mut r, &run, run::THREAD, Some(&"t"), by(2)),
        Err(WriteError::Checked { .. })
    ));
    let thread = Key::new(kind::THREAD, "t").unwrap();
    for key in [&x, &thread] {
        assert!(matches!(
            set_element(&mut r, key, SetField::Repository, "code", true, by(2)),
            Err(WriteError::Checked { .. })
        ));
    }
    // Only the set fields can be formed, so no element reaches another field.
    set_element(&mut r, &x, SetField::Area, "state", true, by(2)).unwrap();
    set_element(&mut r, &x, SetField::Resource, "approved", true, by(2)).unwrap();
    let read = Model::read(&r.state).unwrap();
    assert_eq!(read.items["x"].state, Value::One(ItemState::Backlog));
    assert!(
        read.items["x"].areas.contains("state") && read.items["x"].resources.contains("approved")
    );
    assert!(matches!(
        set(&mut r, &x, field::THREAD, Some(&"t"), by(2)),
        Err(WriteError::Checked { .. })
    ));
    set(&mut r, &x, field::NOTES, Some(&"fine"), by(2)).unwrap();
}

#[test]
fn a_run_only_moves_forward() {
    let mut r = replica("a");
    assert!(matches!(
        set_run_state(&mut r, "r9", RunState::Running, by(1)),
        Err(WriteError::RunTransition { .. })
    ));
    start_run(&mut r, "r1", "t1", by(1)).unwrap();
    set_run_state(&mut r, "r1", RunState::Running, by(2)).unwrap();
    assert!(matches!(
        set_run_state(&mut r, "r1", RunState::Starting, by(3)),
        Err(WriteError::RunTransition { .. })
    ));
    set_run_state(&mut r, "r1", RunState::Stopped, by(3)).unwrap();
    for state in [RunState::Starting, RunState::Running, RunState::Done] {
        assert!(matches!(
            set_run_state(&mut r, "r1", state, by(4)),
            Err(WriteError::RunTransition { .. })
        ));
    }
}

#[test]
fn only_the_user_sets_a_threads_approval_flags_and_the_global_settings() {
    let mut r = replica("a");
    let t = Key::new(kind::THREAD, "t").unwrap();
    let global = Key::new(kind::SETTINGS, crate::schema::GLOBAL).unwrap();
    let fields = [
        (&t, thread::APPROVED),
        (&t, thread::PM_FLAG),
        (&t, thread::AUTONOMY),
        (&t, thread::LAUNCH),
        (&t, thread::PLACEMENT),
        (&global, crate::schema::settings::AUTONOMY),
    ];
    for actor in ["run/boss", "run/worker", "session/mac"] {
        for (key, name) in fields {
            assert_eq!(
                set(&mut r, key, name, Some(&true), By { actor, at: 2 }),
                Err(WriteError::UserOnly { field: name.to_owned(), actor: actor.to_owned() })
            );
        }
    }
    for (key, name) in fields {
        set(&mut r, key, name, Some(&true), by(2)).unwrap();
    }
    items(&mut r, &["x"]);
    assert!(matches!(
        set(&mut r, &item_key("x").unwrap(), field::ARCHIVED, Some(&true), by(3)),
        Err(WriteError::Checked { .. })
    ));
}
