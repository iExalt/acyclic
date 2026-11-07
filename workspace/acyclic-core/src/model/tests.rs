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
fn roles_follow_children_and_the_initiative_above() {
    let mut r = replica("a");
    add_initiative(&mut r, "init", "Init", by(1)).unwrap();
    items(&mut r, &["epic", "plain", "leaf", "loose"]);
    set_parent(&mut r, "epic", Some("init"), by(2)).unwrap();
    set_parent(&mut r, "plain", Some("epic"), by(2)).unwrap();
    set_parent(&mut r, "leaf", Some("plain"), by(2)).unwrap();
    set_parent(&mut r, "loose", Some("init"), by(2)).unwrap();
    // A gate older versions stored is ignored, so it makes no other role.
    r.write(&item_key("plain").unwrap(), "gate", "actor/user", 3, Some(json!("ships"))).unwrap();
    let m = model(&r);
    assert_eq!(m.role("epic"), Role::Epic);
    // Deeper than an epic is a task, with or without children; a childless item under an initiative is one too.
    assert_eq!(m.role("plain"), Role::Task);
    assert_eq!(m.role("leaf"), Role::Task);
    assert_eq!(m.role("loose"), Role::Task);
    assert_eq!(json!(Role::Epic), json!("epic"));
    assert_eq!(json!(Role::Task), json!("task"));
    assert_eq!(m.initiative_of("leaf"), Some("init"));
    assert_eq!(m.initiatives_of("init"), Vec::<&str>::new());
    // With no initiative above, an item with children is where an initiative's work sits, and the rest are tasks.
    let mut r = replica("b");
    items(&mut r, &["alone", "top", "kid"]);
    set_parent(&mut r, "kid", Some("top"), by(2)).unwrap();
    let m = model(&r);
    assert_eq!(
        (m.role("alone"), m.role("top"), m.role("kid")),
        (Role::Task, Role::Epic, Role::Task)
    );
    assert_eq!(m.initiative_of("kid"), None);
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
    assert_eq!(m.role("x"), Role::Task);
    link(&mut a, "x", "y", by(3)).unwrap();
    assert_eq!(model(&a).waits_for_cycles(), Vec::<Vec<String>>::new());
}

#[test]
fn waits_for_cycles_expand_parent_links_to_structural_leaf_cycles() {
    let mut r = replica("a");
    items(&mut r, &["p", "d", "x", "upstream"]);
    set_parent(&mut r, "d", Some("p"), by(2)).unwrap();
    link(&mut r, "p", "x", by(3)).unwrap();
    link(&mut r, "x", "d", by(3)).unwrap();
    link(&mut r, "upstream", "d", by(3)).unwrap();

    assert_eq!(model(&r).waits_for_cycles(), vec![vec!["d".to_owned(), "x".to_owned()]]);

    let mut r = replica("b");
    items(&mut r, &["q", "a", "b", "x"]);
    set_parent(&mut r, "a", Some("q"), by(2)).unwrap();
    set_parent(&mut r, "b", Some("q"), by(2)).unwrap();
    link(&mut r, "x", "q", by(3)).unwrap();
    link(&mut r, "a", "x", by(3)).unwrap();
    // The target parent expands to both leaves, but only `a` points back at x.
    assert_eq!(model(&r).waits_for_cycles(), vec![vec!["a".to_owned(), "x".to_owned()]]);
}

#[test]
fn waits_for_cycles_include_finished_parked_and_canceled_leaves() {
    let mut r = replica("a");
    items(&mut r, &["p", "d", "x"]);
    set_parent(&mut r, "d", Some("p"), by(2)).unwrap();
    link(&mut r, "p", "x", by(3)).unwrap();
    link(&mut r, "x", "d", by(3)).unwrap();
    let expected = vec![vec!["d".to_owned(), "x".to_owned()]];
    let mut snapshot = model(&r);
    for state in [ItemState::Done, ItemState::Parked, ItemState::Canceled] {
        snapshot.items.get_mut("d").unwrap().state = Value::One(state);
        assert_eq!(snapshot.waits_for_cycles(), expected);
    }
}

#[test]
fn read_wait_cycles_exclude_finished_leaves_but_keep_parked_and_conflicted_ones() {
    let mut r = replica("a");
    items(&mut r, &["p", "d", "x"]);
    set_parent(&mut r, "d", Some("p"), by(2)).unwrap();
    link(&mut r, "p", "x", by(3)).unwrap();
    link(&mut r, "x", "d", by(3)).unwrap();
    let expected = vec![vec!["d".to_owned(), "x".to_owned()]];
    let mut snapshot = model(&r);
    assert_eq!(snapshot.read_wait_cycles(), expected);
    snapshot.items.get_mut("d").unwrap().state = Value::One(ItemState::Parked);
    assert_eq!(snapshot.read_wait_cycles(), expected);
    snapshot.items.get_mut("d").unwrap().state =
        Value::Conflict(vec![Some(ItemState::Backlog), Some(ItemState::Done)]);
    assert_eq!(snapshot.read_wait_cycles(), expected);
    snapshot.items.get_mut("d").unwrap().state = Value::One(ItemState::Done);
    assert_eq!(snapshot.read_wait_cycles(), Vec::<Vec<String>>::new());
    snapshot.items.get_mut("d").unwrap().state = Value::One(ItemState::Canceled);
    assert_eq!(snapshot.read_wait_cycles(), Vec::<Vec<String>>::new());
}

#[test]
fn a_conflicted_parent_counts_as_each_of_its_values() {
    let (mut a, mut b) = pair(&["root", "p1", "p2", "child", "x"]);
    for replica in [&mut a, &mut b] {
        set_parent(replica, "p1", Some("root"), by(2)).unwrap();
        set_parent(replica, "p2", Some("root"), by(2)).unwrap();
    }
    set_parent(&mut a, "child", Some("p1"), by(2)).unwrap();
    set_parent(&mut b, "child", Some("p2"), by(2)).unwrap();
    a.join(&b.state).unwrap();
    let m = model(&a);
    assert!(m.items["child"].parent.is_conflict());
    let mut ancestors = m.ancestors("child");
    ancestors.sort_unstable();
    assert_eq!(ancestors, vec!["p1", "p2", "root"]);
    assert_eq!((m.children("p1"), m.children("p2")), (vec!["child"], vec!["child"]));
    assert_eq!(m.progress("root"), Some(Progress { done: 0, canceled: 0, total: 1 }));
    link(&mut a, "root", "x", by(3)).unwrap();
    link(&mut a, "x", "child", by(3)).unwrap();
    assert_eq!(model(&a).waits_for_cycles(), vec![vec!["child".to_owned(), "x".to_owned()]]);
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
    link(&mut r, "a", "b", by(2)).unwrap();
    link(&mut r, "b", "c", by(2)).unwrap();
    let err = link(&mut r, "c", "a", by(3)).unwrap_err();
    assert_eq!(err, WriteError::WaitsForCycle(vec!["a".into(), "b".into(), "c".into()]));
    assert_eq!(link(&mut r, "a", "a", by(3)), Err(WriteError::SelfLink("a".into())));
}

#[test]
fn a_link_of_a_removed_kind_is_refused_on_read_and_listed_by_a_lenient_read() {
    for kind in ["relates", "after"] {
        let mut r = replica("a");
        items(&mut r, &["a", "b"]);
        let key = Key::new(kind::LINK, &format!("{kind}-1-a-b")).unwrap();
        for (name, value) in [("from", "a"), ("to", "b"), ("kind", kind)] {
            r.write(&key, name, "actor/user", 2, Some(json!(value))).unwrap();
        }
        r.write(&key, "present", "actor/user", 2, Some(json!(true))).unwrap();
        let err = Model::read(&r.state).unwrap_err().to_string();
        assert!(err.contains(&format!("kind on link {kind}-1-a-b")), "{err}");
        assert!(err.contains(&format!("`{kind}`")), "{err}");
        let (lenient, unreadable) = Model::read_lenient(&r.state);
        assert_eq!(unreadable.into_iter().map(|(key, _)| key).collect::<Vec<_>>(), [key]);
        assert_eq!(lenient.items.len(), 2);
    }
}

#[test]
fn a_stored_in_progress_state_is_refused_on_read() {
    let mut r = replica("a");
    items(&mut r, &["a"]);
    let key = item_key("a").unwrap();
    r.write(&key, field::STATE, "actor/user", 2, Some(json!("in-progress"))).unwrap();
    let err = Model::read(&r.state).unwrap_err().to_string();
    assert!(err.contains("state on item a") && err.contains("`in-progress`"), "{err}");
}

#[test]
fn a_link_is_removed_once() {
    let mut r = replica("a");
    items(&mut r, &["a", "b"]);
    link(&mut r, "a", "b", by(2)).unwrap();
    assert_eq!(unlink(&mut r, "a", "b", by(3)), Ok(true));
    assert_eq!(model(&r).links, []);
    // Removing it again writes nothing.
    assert_eq!(unlink(&mut r, "a", "b", by(4)), Ok(false));
}

#[test]
fn a_waits_for_cycle_made_by_a_join_is_derived() {
    let (mut a, mut b) = pair(&["x", "y"]);
    link(&mut a, "x", "y", by(2)).unwrap();
    link(&mut b, "y", "x", by(2)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(model(&a).waits_for_cycles(), vec![vec!["x".to_owned(), "y".to_owned()]]);
}

#[test]
fn one_link_added_twice_is_one_link_and_a_re_add_beats_a_removal() {
    let (mut a, mut b) = pair(&["x", "y"]);
    link(&mut a, "x", "y", by(2)).unwrap();
    link(&mut b, "x", "y", by(2)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(model(&a).links.len(), 1);
    b.join(&a.state).unwrap();
    unlink(&mut a, "x", "y", by(3)).unwrap();
    assert_eq!(model(&a).links, Vec::new());
    link(&mut b, "x", "y", by(3)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(model(&a).links.len(), 1, "the concurrent re-add wins");
}

#[test]
fn link_ids_are_unambiguous() {
    let one = link_key("a-b", "c").unwrap();
    let two = link_key("a", "b-c").unwrap();
    assert_ne!(one, two);
}

#[test]
fn names_follow_the_id_rule() {
    let mut r = replica("a");
    for bad in ["a/b", "Engine", "", ".hidden"] {
        assert_eq!(item_key(bad), Err(WriteError::InvalidName(bad.to_owned())));
    }
    let long = "a".repeat(MAX_ID_LEN + 1);
    assert!(matches!(add_item(&mut r, &long, "t", by(2)), Err(WriteError::InvalidName(_))));
    add_item(&mut r, &"a".repeat(MAX_ID_LEN), "t", by(2)).unwrap();
}

#[test]
fn a_conflicted_thread_puts_an_item_in_neither() {
    let (mut a, mut b) = pair(&["x", "y"]);
    let thread_key = |id| Key::new(kind::THREAD, id).unwrap();
    set(&mut a, &thread_key("t1"), thread::TITLE, Some(&"one"), by(2)).unwrap();
    put(&mut a, &item_key("x").unwrap(), field::THREAD, Some(&"t1"), by(2)).unwrap();
    put(&mut a, &item_key("y").unwrap(), field::THREAD, Some(&"t1"), by(2)).unwrap();
    put(&mut b, &item_key("y").unwrap(), field::THREAD, Some(&"t2"), by(2)).unwrap();
    a.join(&b.state).unwrap();
    let m = model(&a);
    assert_eq!(m.members("t1"), vec!["x"]);
    assert_eq!(m.members("t2"), Vec::<&str>::new());
    assert_eq!(m.thread_of("y"), None);
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
    set_state(&mut a, "x", ItemState::Backlog, by(40)).unwrap();
    assert_eq!(model(&a).items["x"].done_at, None);
}

#[test]
fn equal_concurrent_values_read_as_one_and_unreadable_ones_fail() {
    let (mut a, mut b) = pair(&["x"]);
    let key = item_key("x").unwrap();
    set(&mut a, &key, field::NOTES, Some(&"same"), by(2)).unwrap();
    set(&mut b, &key, field::NOTES, Some(&"same"), by(2)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(model(&a).items["x"].notes, Value::One("same".to_owned()));
    a.write(&key, field::STATE, "actor/user", 3, Some(json!("from-the-future"))).unwrap();
    assert!(matches!(Model::read(&a.state), Err(Error::Format(_))));
}

fn names<T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(
    all: &[T],
) -> Vec<String> {
    all.iter()
        .map(|value| {
            let name = serde_json::to_value(value).unwrap();
            assert_eq!(&serde_json::from_value::<T>(name.clone()).unwrap(), value);
            name.as_str().unwrap().to_owned()
        })
        .collect()
}

#[test]
fn every_variant_of_a_value_enum_round_trips() {
    let states = [ItemState::Backlog, ItemState::Parked, ItemState::Done, ItemState::Canceled];
    // Stored as older versions read it, and shown as `open`; `canceled` is new, so older versions can't read it.
    assert_eq!(names(&states), ["backlog", "parked", "done", "canceled"]);
    assert_eq!(states.map(ItemState::word), ["open", "parked", "done", "canceled"]);
}

#[test]
fn a_parent_reads_as_its_leaves_make_it_unless_parked_by_hand() {
    let mut r = replica("a");
    items(&mut r, &["top", "mid", "a", "b", "c"]);
    for (child, parent) in [("mid", "top"), ("a", "mid"), ("b", "mid"), ("c", "top")] {
        set_parent(&mut r, child, Some(parent), by(2)).unwrap();
    }
    let progress = |r: &Replica, id: &str| model(r).progress(id).unwrap();
    let reads = |done, total| Progress { done, canceled: 0, total };
    // Leaves count at every depth, and a leaf has no progress.
    assert_eq!(progress(&r, "top"), reads(0, 3));
    assert_eq!(model(&r).progress("a"), None);
    set_state(&mut r, "a", ItemState::Done, by(3)).unwrap();
    assert_eq!(progress(&r, "top"), reads(1, 3));
    for leaf in ["b", "c"] {
        set_state(&mut r, leaf, ItemState::Done, by(4)).unwrap();
    }
    assert_eq!(progress(&r, "top"), reads(3, 3));
    assert_eq!(progress(&r, "mid"), reads(2, 2));
    assert!(model(&r).reads_done("top") && !model(&r).items["top"].is_done());
    // Parked by hand, a parent's own state holds: it doesn't read done.
    set_state(&mut r, "top", ItemState::Parked, by(5)).unwrap();
    assert!(!model(&r).reads_done("top") && model(&r).reads_done("mid"));
    // Reopening a leaf brings its parents back; a state reopening left on a parent counts for nothing.
    set_state(&mut r, "top", ItemState::Backlog, by(6)).unwrap();
    set_state(&mut r, "b", ItemState::Backlog, by(6)).unwrap();
    set_state(&mut r, "mid", ItemState::Backlog, by(6)).unwrap();
    assert_eq!(progress(&r, "top"), reads(2, 3));
    assert!(!model(&r).reads_done("top"));
    // A done stored on a parent, as older versions wrote, counts for nothing at any depth.
    set_state(&mut r, "top", ItemState::Done, by(7)).unwrap();
    set_state(&mut r, "mid", ItemState::Done, by(7)).unwrap();
    assert_eq!(progress(&r, "top"), reads(2, 3));
    assert!(!model(&r).reads_done("mid") && !model(&r).reads_done("top"));
}

#[test]
fn canceled_leaves_leave_the_count_and_a_parent_of_nothing_else_reads_canceled() {
    let mut r = replica("a");
    items(&mut r, &["top", "a", "b"]);
    for child in ["a", "b"] {
        set_parent(&mut r, child, Some("top"), by(2)).unwrap();
    }
    let read = |r: &Replica| model(r);
    set_state(&mut r, "a", ItemState::Canceled, by(3)).unwrap();
    assert_eq!(read(&r).progress("top"), Some(Progress { done: 0, canceled: 1, total: 1 }));
    assert!(read(&r).items["a"].is_canceled() && read(&r).items["a"].is_finished());
    assert!(!read(&r).items["a"].is_done() && read(&r).reads_finished("a"));
    // The parent rolls up from its other leaves.
    assert!(!read(&r).reads_done("top") && !read(&r).reads_canceled("top"));
    set_state(&mut r, "b", ItemState::Done, by(4)).unwrap();
    assert!(read(&r).reads_done("top") && !read(&r).reads_canceled("top"));
    // All canceled reads canceled, never done.
    set_state(&mut r, "b", ItemState::Canceled, by(5)).unwrap();
    assert!(read(&r).reads_canceled("top") && !read(&r).reads_done("top"));
    // Canceled by hand, a parent reads canceled whatever its leaves say, and its open leaves count as canceled.
    set_state(&mut r, "b", ItemState::Backlog, by(6)).unwrap();
    set_state(&mut r, "top", ItemState::Canceled, by(6)).unwrap();
    assert!(read(&r).reads_canceled("top") && !read(&r).reads_done("top"));
    assert_eq!(read(&r).progress("top"), Some(Progress { done: 0, canceled: 2, total: 0 }));
    // The parent's state doesn't turn its completed leaves into canceled work.
    set_state(&mut r, "a", ItemState::Done, by(7)).unwrap();
    set_state(&mut r, "b", ItemState::Done, by(7)).unwrap();
    assert_eq!(read(&r).progress("top"), Some(Progress { done: 2, canceled: 0, total: 2 }));
    assert!(read(&r).reads_canceled("top") && !read(&r).reads_done("top"));
    // A state in conflict between done and canceled is neither.
    let (mut left, mut right) = pair(&["c"]);
    set_state(&mut left, "c", ItemState::Done, by(3)).unwrap();
    set_state(&mut right, "c", ItemState::Canceled, by(3)).unwrap();
    left.join(&right.state).unwrap();
    let conflicted = &model(&left).items["c"];
    assert!(conflicted.state.is_conflict() && !conflicted.is_done() && !conflicted.is_canceled());
    assert!(!conflicted.is_finished());
}

#[test]
fn a_conflicted_leaf_is_not_done_and_a_thread_holding_a_parent_finishes_with_its_leaves() {
    let (mut a, mut b) = pair(&["p", "x", "y"]);
    for replica in [&mut a, &mut b] {
        set_parent(replica, "x", Some("p"), by(2)).unwrap();
        set_parent(replica, "y", Some("p"), by(2)).unwrap();
    }
    set_state(&mut a, "x", ItemState::Done, by(3)).unwrap();
    set_state(&mut b, "x", ItemState::Done, by(3)).unwrap();
    set_state(&mut a, "y", ItemState::Done, by(3)).unwrap();
    set_state(&mut b, "y", ItemState::Parked, by(3)).unwrap();
    a.join(&b.state).unwrap();
    assert!(model(&a).items["y"].state.is_conflict());
    assert_eq!(model(&a).progress("p"), Some(Progress { done: 1, canceled: 0, total: 2 }));
    // A parent reads as done once its leaves are.
    assert!(!model(&a).reads_done("p"));
    set_state(&mut a, "y", ItemState::Done, by(5)).unwrap();
    assert!(model(&a).reads_done("p"));
}

#[test]
fn a_write_holds_only_a_lone_equal_value_or_for_a_clear_nothing() {
    assert!(Value::<i32>::None.holds(None));
    assert!(!Value::<i32>::None.holds(Some(&1)));
    assert!(Value::One(1).holds(Some(&1)));
    assert!(!Value::One(1).holds(Some(&2)));
    assert!(!Value::One(1).holds(None));
    // A conflict holds neither side, so writing either settles it.
    let conflict = Value::Conflict(vec![Some(1), None]);
    assert!(!conflict.holds(Some(&1)) && !conflict.holds(None));
}

#[test]
fn a_deleted_record_is_gone_from_the_model_but_its_id_is_remembered() {
    let (mut a, mut b) = pair(&["kept", "gone"]);
    let thread = Key::new(kind::THREAD, "t").unwrap();
    set(&mut a, &thread, thread::TITLE, Some(&"T"), by(2)).unwrap();
    // Two replicas retitle the item apart, then one deletes it: its conflict goes with it.
    set(&mut a, &item_key("gone").unwrap(), field::TITLE, Some(&"A"), by(3)).unwrap();
    set(&mut b, &item_key("gone").unwrap(), field::TITLE, Some(&"B"), by(3)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(model(&a).conflicts().count(), 1);
    crate::delete::delete_item(&mut a, "gone", by(4)).unwrap();
    crate::delete::delete_thread(&mut a, "t", by(4)).unwrap();
    let read = model(&a);
    assert!(!read.items.contains_key("gone") && !read.threads.contains_key("t"));
    assert!(read.is_deleted(&item_key("gone").unwrap()) && read.is_deleted(&thread));
    assert!(!read.is_deleted(&item_key("kept").unwrap()));
    assert_eq!(read.conflicts().count(), 0);
}

/// `canceled` is a stored value older replicas can't read, so nothing writes it on its own: only the explicit edit
/// (the CLI's `--state canceled`, which the app's state select sends), an import that says so, and keeping a side of
/// a conflict. Every file that can name it is listed, so a new writer fails here until it is looked at.
#[test]
fn only_the_explicit_paths_name_the_canceled_state() {
    const MAY_NAME: [&str; 8] = [
        "acyclic-core/src/model.rs",
        "acyclic-core/src/ready.rs",
        "acyclic-core/src/write.rs",
        "acyclic-core/src/needs.rs",
        "acyclic-core/src/import.rs",
        "acyclic-cli/src/args.rs",
        "acyclic-cli/src/edit.rs",
        "acyclic-cli/src/commands.rs",
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").canonicalize().unwrap();
    let mut named = Vec::new();
    let mut dirs = vec![root.clone()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().map(Result::unwrap) {
            let (path, name) = (entry.path(), entry.file_name().to_string_lossy().into_owned());
            if path.is_dir() {
                if !matches!(name.as_str(), "target" | "node_modules" | "tests" | ".git") {
                    dirs.push(path);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs") && name != "tests.rs" {
                let text = std::fs::read_to_string(&path).unwrap();
                // What follows a file's own test module isn't shipped.
                let shipped = text.split("#[cfg(test)]").next().unwrap_or_default();
                if shipped.contains("State::Canceled") {
                    named.push(path.strip_prefix(&root).unwrap().display().to_string());
                }
            }
        }
    }
    named.sort();
    for file in &named {
        assert!(
            MAY_NAME.contains(&file.as_str()),
            "{file} names the canceled state: only the explicit paths may write it ({named:?})"
        );
    }
    assert!(named.len() > 1, "the guard looked at nothing: {named:?}");
}

#[test]
fn an_initiative_is_stored_as_its_kind_title_notes_and_tombstone() {
    let mut r = replica("a");
    add_initiative(&mut r, "garden", "Garden", by(1)).unwrap();
    let key = Key::new(kind::INITIATIVE, "garden").unwrap();
    // Notes are stored as `notes`, with no older name, and read back from it.
    put(&mut r, &key, initiative::NOTES, Some(&"Beds first"), by(2)).unwrap();
    let stored = |r: &Replica| -> serde_json::Value {
        let files = r.state.to_files();
        // Pinned, with the fan-out: a change to the kind's name, its fields or its place moves every record.
        assert_eq!(key.path(), "initiative/cc/garden.json");
        serde_json::from_str(&files[&key.path()]).unwrap()
    };
    let value = |at: u64, value: serde_json::Value| json!([{"actor": "actor/user", "at": at, "dot": ["a", at], "value": value}]);
    assert_eq!(
        stored(&r),
        json!({
            "kind": "initiative", "id": "garden",
            "fields": {"title": value(1, json!("Garden")), "notes": value(2, json!("Beds first"))},
        })
    );
    let m = model(&r);
    assert_eq!(m.initiatives["garden"].title, Value::One("Garden".into()));
    assert_eq!(m.initiatives["garden"].notes, Value::One("Beds first".into()));
    // Deleting sets the tombstone every kind shares, and the initiative is then gone from every reader.
    put(&mut r, &key, initiative::DELETED, Some(&true), by(3)).unwrap();
    assert_eq!(stored(&r)["fields"]["deleted"], value(3, json!(true)));
    let m = model(&r);
    assert!(m.initiatives.is_empty() && m.is_deleted(&key));
}
