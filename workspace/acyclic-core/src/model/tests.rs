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
fn roles_follow_children_and_the_top_level() {
    let mut r = replica("a");
    items(&mut r, &["top", "plain", "leaf"]);
    set_parent(&mut r, "plain", Some("top"), by(2)).unwrap();
    set_parent(&mut r, "leaf", Some("plain"), by(2)).unwrap();
    // A gate older versions stored is ignored, so it makes no other role.
    r.write(&item_key("plain").unwrap(), "gate", "actor/user", 3, Some(json!("ships"))).unwrap();
    let m = model(&r);
    assert_eq!(m.role("top"), Role::Project);
    assert_eq!(m.role("plain"), Role::Group);
    assert_eq!(m.role("leaf"), Role::Leaf);
    assert_eq!(json!(Role::Leaf), json!("item"));
    // An unplaced leaf keeps its leaf role.
    let mut r = replica("b");
    items(&mut r, &["alone"]);
    assert_eq!(model(&r).role("alone"), Role::Leaf);
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
    link(&mut r, "a", "b", by(2)).unwrap();
    link(&mut r, "b", "c", by(2)).unwrap();
    let err = link(&mut r, "c", "a", by(3)).unwrap_err();
    assert_eq!(err, WriteError::LinkCycle(vec!["a".into(), "b".into(), "c".into()]));
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
fn a_link_cycle_made_by_a_join_is_derived() {
    let (mut a, mut b) = pair(&["x", "y"]);
    link(&mut a, "x", "y", by(2)).unwrap();
    link(&mut b, "y", "x", by(2)).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(model(&a).link_cycles(), vec![vec!["x".to_owned(), "y".to_owned()]]);
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
fn a_threads_heads_are_its_open_leaves_that_no_link_orders_after_another_member() {
    let mut r = replica("a");
    items(&mut r, &["a", "b", "c", "d", "p", "x"]);
    for id in ["a", "b", "c", "d"] {
        put(&mut r, &item_key(id).unwrap(), field::THREAD, Some(&"t"), by(2)).unwrap();
    }
    let heads = |r: &Replica| {
        model(r).heads_index().of("t").into_iter().map(str::to_owned).collect::<Vec<_>>()
    };
    assert_eq!(heads(&r), ["a", "b", "c", "d"], "membership alone orders nothing");
    link(&mut r, "b", "a", by(3)).unwrap();
    // Through an item outside the thread, a link orders too.
    link(&mut r, "c", "x", by(3)).unwrap();
    link(&mut r, "x", "a", by(3)).unwrap();
    // A parent's links order what is under it.
    set_parent(&mut r, "d", Some("p"), by(3)).unwrap();
    link(&mut r, "p", "a", by(3)).unwrap();
    assert_eq!(heads(&r), ["a"]);
    set_state(&mut r, "a", ItemState::Done, by(4)).unwrap();
    assert_eq!(heads(&r), ["b", "c", "d"], "a done member orders nothing");
    // A parent in the thread is never a head, and following it follows what is under it.
    put(&mut r, &item_key("p").unwrap(), field::THREAD, Some(&"t"), by(5)).unwrap();
    link(&mut r, "b", "p", by(5)).unwrap();
    assert_eq!(heads(&r), ["c", "d"]);
}

/// Heads as their definition reads, walking every link from each leaf with nothing gathered ahead. A member parent is
/// open until it reads as done, by hand or by its leaves.
fn heads_by_definition<'a>(m: &'a Model, thread: &str) -> Vec<&'a str> {
    let open: Vec<&str> = m.members(thread).into_iter().filter(|id| !m.reads_done(id)).collect();
    let lift = |id: &'a str| std::iter::once(id).chain(m.ancestors(id));
    let follows_another = |leaf: &'a str| {
        let others: BTreeSet<&str> =
            open.iter().filter(|member| **member != leaf).flat_map(|member| lift(member)).collect();
        let mut seen: BTreeSet<&str> = lift(leaf).collect();
        let mut frontier: Vec<&str> = seen.iter().copied().collect();
        while let Some(node) = frontier.pop() {
            for link in m.links.iter().filter(|link| link.from == node) {
                if others.contains(link.to.as_str()) {
                    return true;
                }
                for reached in lift(&link.to) {
                    if seen.insert(reached) {
                        frontier.push(reached);
                    }
                }
            }
        }
        false
    };
    open.iter().copied().filter(|id| m.children(id).is_empty() && !follows_another(id)).collect()
}

#[hegel::test]
fn heads_match_their_definition(tc: hegel::TestCase) {
    use hegel::generators as gs;
    let below = |n: usize| tc.draw(gs::integers::<usize>().max_value(n - 1));
    let n = 2 + below(6);
    let ids: Vec<String> = (0..n).map(|i| format!("i{i}")).collect();
    let mut r = replica("a");
    items(&mut r, &ids.iter().map(String::as_str).collect::<Vec<_>>());
    for (i, id) in ids.iter().enumerate() {
        let parent = below(i + 1);
        if parent < i {
            set_parent(&mut r, id, Some(&ids[parent]), by(2)).unwrap();
        }
        if let Some(thread) = [None, Some("t"), Some("u")][below(3)] {
            put(&mut r, &item_key(id).unwrap(), field::THREAD, Some(&thread), by(2)).unwrap();
        }
        if below(4) == 0 {
            set_state(&mut r, id, ItemState::Done, by(3)).unwrap();
        }
    }
    for _ in 0..below(9) {
        let (from, to) = (below(n), below(n));
        // A link that would close a cycle is refused, as it is for anyone.
        let _ = link(&mut r, &ids[from], &ids[to], by(4));
    }
    let m = model(&r);
    for thread in ["t", "u"] {
        assert_eq!(
            m.heads_index().of(thread),
            heads_by_definition(&m, thread),
            "{thread}: {:?}",
            m.links
        );
    }
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
    let states = [ItemState::Backlog, ItemState::Parked, ItemState::Done];
    assert_eq!(names(&states), ["backlog", "parked", "done"]);
    let progress = [
        ProgressState::Backlog,
        ProgressState::Parked,
        ProgressState::InProgress,
        ProgressState::Done,
    ];
    let progress: Vec<Json> =
        progress.iter().map(|state| serde_json::to_value(state).unwrap()).collect();
    assert_eq!(progress, [json!("backlog"), json!("parked"), json!("in-progress"), json!("done")]);
}

#[test]
fn a_parent_reads_as_its_leaves_make_it_unless_parked_by_hand() {
    let mut r = replica("a");
    items(&mut r, &["top", "mid", "a", "b", "c"]);
    for (child, parent) in [("mid", "top"), ("a", "mid"), ("b", "mid"), ("c", "top")] {
        set_parent(&mut r, child, Some(parent), by(2)).unwrap();
    }
    let progress = |r: &Replica, id: &str| model(r).progress(id).unwrap();
    let reads = |state, done, total| Progress { state, done, total };
    // Leaves count at every depth, and a leaf has no progress.
    assert_eq!(progress(&r, "top"), reads(ProgressState::Backlog, 0, 3));
    assert_eq!(model(&r).progress("a"), None);
    set_state(&mut r, "a", ItemState::Done, by(3)).unwrap();
    assert_eq!(progress(&r, "top"), reads(ProgressState::InProgress, 1, 3));
    for leaf in ["b", "c"] {
        set_state(&mut r, leaf, ItemState::Done, by(4)).unwrap();
    }
    assert_eq!(progress(&r, "top"), reads(ProgressState::Done, 3, 3));
    assert_eq!(progress(&r, "mid"), reads(ProgressState::Done, 2, 2));
    assert!(model(&r).reads_done("top") && !model(&r).items["top"].is_done());
    // Reopening a leaf brings its parents back; a state reopening left on a parent counts for nothing.
    set_state(&mut r, "b", ItemState::Backlog, by(5)).unwrap();
    set_state(&mut r, "mid", ItemState::Backlog, by(5)).unwrap();
    assert_eq!(progress(&r, "top"), reads(ProgressState::InProgress, 2, 3));
    // Parked by hand, a parent's own state holds.
    set_state(&mut r, "top", ItemState::Parked, by(6)).unwrap();
    assert_eq!(progress(&r, "top"), reads(ProgressState::Parked, 2, 3));
    // A done stored on a parent, as older versions wrote, counts for nothing at any depth.
    set_state(&mut r, "top", ItemState::Done, by(7)).unwrap();
    set_state(&mut r, "mid", ItemState::Done, by(7)).unwrap();
    assert_eq!(progress(&r, "top"), reads(ProgressState::InProgress, 2, 3));
    assert!(!model(&r).reads_done("mid"));
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
    assert_eq!(
        model(&a).progress("p").map(|p| (p.state, p.done)),
        Some((ProgressState::InProgress, 1))
    );
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
