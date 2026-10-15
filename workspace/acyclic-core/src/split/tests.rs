use hegel::TestCase;
use hegel::generators as gs;
use serde_json::json;

use super::*;
use crate::model::{By, ItemState, add_item, item_key, link, link_key, set_parent, set_state};
use crate::schema::{item as field, kind, link as link_field, thread};
use crate::state::{Key, Replica, ReplicaId, State};

const USER: By<'static> = By { actor: "actor/user", at: 1 };

fn replica() -> Replica {
    replica_named("mac")
}

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

/// Top `p` with group `g1` of `n1` leaves and group `g2` of `n2` leaves.
fn two_groups(n1: usize, n2: usize) -> Replica {
    let mut r = replica();
    add(&mut r, "p", None);
    for (group, n) in [("g1", n1), ("g2", n2)].into_iter().filter(|(_, n)| *n > 0) {
        add(&mut r, group, Some("p"));
        for i in 0..n {
            add(&mut r, &format!("{group}-{i:02}"), Some(group));
        }
    }
    r
}

fn model(r: &Replica) -> Model {
    Model::read(r.state()).unwrap()
}

fn members(drafts: &[Draft]) -> Vec<(String, usize)> {
    drafts.iter().map(|d| (d.root.clone(), d.members.len())).collect()
}

#[test]
fn a_small_subtree_is_one_draft_and_a_large_one_splits_along_the_tree() {
    let small = plan(&model(&two_groups(5, 6)), None);
    assert_eq!(members(&small), vec![("p".to_owned(), 11)]);
    assert!(small[0].id.starts_with("s-p-") && small[0].title == "Title p");
    let large = plan(&model(&two_groups(20, 20)), None);
    assert_eq!(members(&large), vec![("g1".to_owned(), 20), ("g2".to_owned(), 20)]);
    assert!(large.iter().all(|d| !d.merged_cycle));
    assert_eq!(plan(&model(&two_groups(20, 20)), None), large, "deterministic");
    assert_ne!(large[0].id, large[1].id);
    assert_eq!(plan(&model(&two_groups(20, 20)), Some("g2")).len(), 1, "a root limits the split");
}

#[test]
fn linked_leaves_stay_together_across_groups() {
    let mut r = two_groups(20, 20);
    link(&mut r, "g1-00", "g2-00", LinkKind::After, USER).unwrap();
    let drafts = plan(&model(&r), None);
    let together = drafts.iter().find(|d| d.members.contains("g1-00")).unwrap();
    assert!(together.members.contains("g2-00"), "{drafts:?}");
    assert_eq!(together.root, "p");
    // `relates` orders nothing, so it ties nothing.
    let mut r = two_groups(20, 20);
    link(&mut r, "g1-00", "g2-00", LinkKind::Relates, USER).unwrap();
    assert_eq!(
        members(&plan(&model(&r), None)),
        vec![("g1".to_owned(), 20), ("g2".to_owned(), 20)]
    );
}

#[test]
fn an_overfull_node_packs_even_drafts() {
    let drafts = plan(&model(&two_groups(26, 0)), None);
    assert_eq!(members(&drafts), vec![("g1".to_owned(), 13), ("g1".to_owned(), 13)]);
}

#[test]
fn a_subtree_under_two_parents_waits_for_the_conflict() {
    let mut a = replica();
    for top in ["a", "b"] {
        add(&mut a, top, None);
    }
    add(&mut a, "g", Some("a"));
    for leaf in ["l0", "l1"] {
        add(&mut a, leaf, Some("g"));
    }
    let mut b = replica_named("pi");
    b.join(a.state()).unwrap();
    set_parent(&mut b, "g", Some("b"), USER).unwrap();
    set_parent(&mut a, "g", None, USER).unwrap();
    set_parent(&mut a, "g", Some("a"), USER).unwrap();
    a.join(b.state()).unwrap();
    assert!(model(&a).items["g"].parent.is_conflict());
    assert_eq!(plan(&model(&a), None), Vec::<Draft>::new());
}

#[test]
fn only_open_unarchived_leaves_without_a_thread_are_placed() {
    let mut r = two_groups(3, 0);
    set_state(&mut r, "g1-00", ItemState::Done, USER).unwrap();
    write(&mut r, &item_key("g1-01").unwrap(), field::THREAD, json!("t"));
    let drafts = plan(&model(&r), None);
    assert_eq!(drafts[0].members, BTreeSet::from(["g1-02".to_owned()]));
    write(&mut r, &item_key("p").unwrap(), field::ARCHIVED, json!(true));
    assert_eq!(plan(&model(&r), None), Vec::<Draft>::new());
}

#[test]
fn conflicting_unapproved_split_threads_are_healed_but_approved_ones_are_left() {
    let mut a = two_groups(1, 0);
    let mut b = replica_named("pi");
    b.join(a.state()).unwrap();
    let key = item_key("g1-00").unwrap();
    write(&mut a, &key, field::THREAD, json!("s-p-aaaaaa"));
    write(&mut b, &key, field::THREAD, json!("s-p-bbbbbb"));
    a.join(b.state()).unwrap();
    assert!(model(&a).items["g1-00"].thread.is_conflict());
    assert_eq!(plan(&model(&a), None).len(), 1, "two unapproved split threads heal");
    write(&mut a, &Key::new(kind::THREAD, "s-p-bbbbbb").unwrap(), thread::APPROVED, json!(true));
    assert_eq!(plan(&model(&a), None), Vec::<Draft>::new(), "an approved value stays a clash");
}

#[test]
fn a_block_on_a_group_counts_for_its_leaves_and_drafts_never_deadlock() {
    // `g1` as a whole waits on `g2-00`, which ties 27 leaves: too many to keep whole, so they are cut along the tree.
    let mut r = two_groups(26, 5);
    link(&mut r, "g1", "g2-00", LinkKind::Blocks, USER).unwrap();
    let drafts = plan(&model(&r), None);
    assert!(drafts.iter().all(|d| !d.merged_cycle), "{drafts:?}");
    assert_no_deadlock(&model(&r), &drafts);
    // Now `g2-01` waits on `g1-00` too: cut along the tree, `g1`'s drafts and `g2`'s would wait on each other, so
    // they are cut again along the order the links give, still within the cap.
    link(&mut r, "g2-01", "g1-00", LinkKind::Blocks, USER).unwrap();
    let drafts = plan(&model(&r), None);
    assert_no_deadlock(&model(&r), &drafts);
    assert!(drafts.iter().all(|d| !d.merged_cycle && d.members.len() <= CAP), "{drafts:?}");
}

#[test]
fn drafts_waiting_on_each_other_across_top_level_items_share_one_and_say_so() {
    let mut r = replica();
    for (top, leaves) in [("a", ["a0", "a1"]), ("b", ["b0", "b1"])] {
        add(&mut r, top, None);
        for leaf in leaves {
            add(&mut r, leaf, Some(top));
        }
    }
    link(&mut r, "a0", "b0", LinkKind::Blocks, USER).unwrap();
    assert_eq!(plan(&model(&r), None).len(), 2, "links never tie two top-level items");
    link(&mut r, "b1", "a1", LinkKind::Blocks, USER).unwrap();
    let drafts = plan(&model(&r), None);
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    assert!(!drafts[0].merged_cycle && drafts[0].cross_top && drafts[0].members.len() == 4);
}

#[test]
fn pieces_waiting_on_each_other_share_a_draft_that_says_so() {
    // A join can leave two leaves in different top-level items waiting on each other; no order can part them.
    let mut r = replica();
    for top in ["a", "b"] {
        add(&mut r, top, None);
        add(&mut r, &format!("{top}0"), Some(top));
    }
    for (from, to) in [("a0", "b0"), ("b0", "a0")] {
        let key = link_key(from, to, LinkKind::Blocks).unwrap();
        write(&mut r, &key, link_field::FROM, json!(from));
        write(&mut r, &key, link_field::TO, json!(to));
        write(&mut r, &key, link_field::KIND, json!("blocks"));
        write(&mut r, &key, link_field::PRESENT, json!(true));
    }
    let drafts = plan(&model(&r), None);
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    assert!(drafts[0].merged_cycle && drafts[0].cross_top, "{drafts:?}");
}

#[test]
fn a_cycle_through_an_existing_thread_is_named_not_merged() {
    let mut r = replica();
    add(&mut r, "p", None);
    for leaf in ["x", "x2", "y", "z"] {
        add(&mut r, leaf, Some("p"));
    }
    for leaf in ["x", "x2"] {
        write(&mut r, &item_key(leaf).unwrap(), field::THREAD, json!("t"));
    }
    link(&mut r, "y", "x", LinkKind::Blocks, USER).unwrap();
    assert_eq!(plan(&model(&r), None)[0].waits_with, BTreeSet::new());
    link(&mut r, "x2", "z", LinkKind::Blocks, USER).unwrap();
    let drafts = plan(&model(&r), None);
    assert_eq!(members(&drafts), vec![("p".to_owned(), 2)]);
    assert_eq!(drafts[0].waits_with, BTreeSet::from(["t".to_owned()]));
}

#[test]
fn ids_fit_and_titles_name_a_shared_area() {
    let long = "x".repeat(MAX_ID_LEN);
    let id = draft_id(&long, &BTreeSet::from(["a".to_owned()]), 0);
    assert!(id.len() <= MAX_ID_LEN && id.starts_with("s-xxx"));
    assert_ne!(id, draft_id(&long, &BTreeSet::from(["b".to_owned()]), 0));
    let mut r = two_groups(2, 0);
    for leaf in ["g1-00", "g1-01"] {
        crate::model::set_element(
            &mut r,
            &item_key(leaf).unwrap(),
            crate::model::SetField::Area,
            "paint",
            true,
            USER,
        )
        .unwrap();
    }
    assert_eq!(plan(&model(&r), None)[0].title, "Title p · paint");
}

/// Random forests with links, threads, done and archived items.
fn forest(tc: &TestCase) -> Replica {
    let mut r = replica();
    let count = tc.draw(gs::integers::<usize>().min_value(1).max_value(120));
    let below = |n: usize| tc.draw(gs::integers::<usize>().max_value(n - 1));
    // A few top-level items, so subtrees grow past the cap.
    let tops = 1 + below(4);
    for i in 0..count {
        add(&mut r, &format!("i{i}"), None);
        // An earlier item, so no parent cycle; written directly, since `set_parent` reads the model to check.
        if i >= tops {
            write(
                &mut r,
                &item_key(&format!("i{i}")).unwrap(),
                field::PARENT,
                json!(format!("i{}", below(i))),
            );
        }
        match below(12) {
            0 => set_state(&mut r, &format!("i{i}"), ItemState::Done, USER).unwrap(),
            1 => write(&mut r, &item_key(&format!("i{i}")).unwrap(), field::THREAD, json!("t")),
            2 => write(&mut r, &item_key(&format!("i{i}")).unwrap(), field::ARCHIVED, json!(true)),
            _ => {},
        }
    }
    for _ in 0..below(count + 1) {
        let kinds = [LinkKind::Blocks, LinkKind::After, LinkKind::Relates];
        // A third of ends are among the first items, which hold the most below them, so components outgrow the cap.
        let end = || if below(3) == 0 { below(count.min(8)) } else { below(count) };
        let (from, to) = (format!("i{}", end()), format!("i{}", end()));
        // Written as a join would leave them, which can close an ordering cycle; `link` would refuse one, and
        // checking costs a model read per link.
        if from != to {
            let kind = kinds[below(3)];
            let key = link_key(&from, &to, kind).unwrap();
            write(&mut r, &key, link_field::FROM, json!(from));
            write(&mut r, &key, link_field::TO, json!(to));
            write(&mut r, &key, link_field::KIND, serde_json::to_value(kind).unwrap());
            write(&mut r, &key, link_field::PRESENT, json!(true));
        }
    }
    // Two replicas move a few items at once, leaving their parents in conflict.
    let mut other = replica_named("pi");
    other.join(r.state()).unwrap();
    for _ in 0..below(3) {
        if count > tops {
            let i = tops + below(count - tops);
            let key = item_key(&format!("i{i}")).unwrap();
            write(&mut r, &key, field::PARENT, json!(format!("i{}", below(i))));
            write(&mut other, &key, field::PARENT, json!(format!("i{}", below(i))));
        }
    }
    r.join(other.state()).unwrap();
    r
}

/// A candidate whose place is settled: no item on its line, it included, has a parent in conflict.
fn placeable(model: &Model, id: &str) -> bool {
    candidate(model, id)
        && std::iter::once(id)
            .chain(model.ancestors(id))
            .all(|node| !model.items[node].parent.is_conflict())
}

/// The placeable leaves' components: tied by links that order work, projected onto leaves, within one top-level
/// subtree.
fn components(model: &Model) -> Vec<BTreeSet<String>> {
    let leaves = |id: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![id.to_owned()];
        while let Some(next) = stack.pop() {
            let children = model.children(&next);
            if children.is_empty() {
                if placeable(model, &next) {
                    out.push(next);
                }
            } else {
                stack.extend(children.into_iter().map(str::to_owned));
            }
        }
        out
    };
    let top: BTreeMap<&str, &str> = model
        .items
        .keys()
        .map(|id| (id.as_str(), model.ancestors(id).last().copied().unwrap_or(id)))
        .collect();
    let mut owner: BTreeMap<String, usize> = model
        .items
        .keys()
        .filter(|id| placeable(model, id) && !model.is_archived(top[id.as_str()]))
        .enumerate()
        .map(|(i, id)| (id.clone(), i))
        .collect();
    for link in model.links.iter().filter(|link| link.kind.orders()) {
        // Within each top-level subtree, every end joins the first end's component.
        let mut first: BTreeMap<&str, usize> = BTreeMap::new();
        for leaf in leaves(&link.from).into_iter().chain(leaves(&link.to)) {
            let Some(&j) = owner.get(&leaf) else { continue };
            let i = *first.entry(top[leaf.as_str()]).or_insert(j);
            owner.values_mut().filter(|o| **o == j).for_each(|o| *o = i);
        }
    }
    let mut groups: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for (id, i) in owner {
        groups.entry(i).or_default().insert(id);
    }
    groups.into_values().collect()
}

/// (a) No draft waits on another in a cycle through `blocks` links, a link on a group counting for its leaves.
fn assert_no_deadlock(m: &Model, drafts: &[Draft]) {
    let mut drafts_under: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    for (i, draft) in drafts.iter().enumerate() {
        for member in &draft.members {
            for node in std::iter::once(member.as_str()).chain(m.ancestors(member)) {
                drafts_under.entry(node).or_default().insert(i);
            }
        }
    }
    let mut edges = Vec::new();
    for link in m.links.iter().filter(|l| l.kind == LinkKind::Blocks) {
        let none = BTreeSet::new();
        let under = |id: &str| drafts_under.get(id).unwrap_or(&none).clone();
        for to in under(&link.to) {
            for from in under(&link.from).into_iter().filter(|from| *from != to) {
                edges.push((to, from));
            }
        }
    }
    assert_eq!(
        crate::graph::cycles(&edges),
        Vec::<Vec<usize>>::new(),
        "(a) drafts wait on each other"
    );
}

#[hegel::test(test_cases = 200)]
fn splits_keep_their_promises(tc: TestCase) {
    let mut r = forest(&tc);
    let m = model(&r);
    let drafts = plan(&m, None);
    assert_eq!(plan(&m, None), drafts, "(e) deterministic");
    // Every candidate in a placed subtree is placed once.
    let placed: Vec<&String> = drafts.iter().flat_map(|d| &d.members).collect();
    let unique: BTreeSet<&String> = placed.iter().copied().collect();
    assert_eq!(placed.len(), unique.len(), "a leaf in two drafts");
    for draft in &drafts {
        for member in &draft.members {
            assert!(placeable(&m, member), "(f) {member} wasn't free to place");
            if !draft.cross_top {
                assert!(
                    member == &draft.root || m.ancestors(member).contains(&draft.root.as_str()),
                    "(b) {member} isn't under {}",
                    draft.root
                );
            }
        }
        assert!(draft.merged_cycle || draft.members.len() <= CAP, "(c) {} over the cap", draft.id);
    }
    // (d) a component that fits stays in one draft.
    for component in components(&m).into_iter().filter(|c| c.len() <= CAP) {
        let drafts_of: BTreeSet<&str> = drafts
            .iter()
            .filter(|d| d.members.iter().any(|member| component.contains(member)))
            .map(|d| d.id.as_str())
            .collect();
        assert!(drafts_of.len() <= 1, "(d) {component:?} cut across {drafts_of:?}");
    }
    // (a) no draft waits on another in a cycle.
    assert_no_deadlock(&m, &drafts);
    // (g) once applied, nothing is left to split.
    for draft in &drafts {
        for member in &draft.members {
            write(&mut r, &item_key(member).unwrap(), field::THREAD, json!(draft.id));
        }
    }
    assert_eq!(plan(&model(&r), None), Vec::<Draft>::new(), "(g) a second split found work");
}

fn at(at: i64) -> By<'static> {
    By { actor: USER.actor, at }
}

#[test]
fn concurrent_splits_on_two_replicas_converge_after_one_more() {
    let mut a = two_groups(2, 0);
    let mut b = replica_named("pi");
    b.join(a.state()).unwrap();
    add_item(&mut a, "mine", "Mine", at(2)).unwrap();
    set_parent(&mut a, "mine", Some("g1"), at(2)).unwrap();
    add_item(&mut b, "yours", "Yours", at(2)).unwrap();
    set_parent(&mut b, "yours", Some("g1"), at(2)).unwrap();
    apply(&mut a, None, at(3)).unwrap();
    apply(&mut b, None, at(3)).unwrap();
    a.join(b.state()).unwrap();
    b.join(a.state()).unwrap();
    assert!(model(&a).items["g1-00"].thread.is_conflict(), "each replica placed the shared leaves");
    // Both heal at once, to the same draft, so their writes agree.
    let healed = apply(&mut a, None, at(4)).unwrap();
    assert_eq!(apply(&mut b, None, at(5)).unwrap(), healed);
    a.join(b.state()).unwrap();
    let m = model(&a);
    assert_eq!(plan(&m, None), Vec::<Draft>::new());
    for leaf in ["g1-00", "g1-01"] {
        assert_eq!(m.thread_of(leaf), Some(healed[0].id.as_str()), "{leaf}");
    }
    assert!(m.thread_of("mine").is_some() && m.thread_of("yours").is_some());
}

#[test]
fn a_draft_never_writes_into_a_thread_that_exists() {
    let mut r = two_groups(2, 0);
    let first = plan(&model(&r), None);
    let taken = Key::new(kind::THREAD, &first[0].id).unwrap();
    write(&mut r, &taken, thread::TITLE, json!("Emptied"));
    write(&mut r, &taken, thread::APPROVED, json!(true));
    let applied = apply(&mut r, None, USER).unwrap();
    assert_eq!(applied[0].members, first[0].members);
    assert_ne!(applied[0].id, first[0].id);
    assert_eq!(model(&r).members(&first[0].id), Vec::<&str>::new());
    assert_eq!(model(&r).threads[&applied[0].id].title.one().map(String::as_str), Some("Title p"));
}

fn place_one(r: &mut Replica, item: &str, budget: Option<usize>, actor: &'static str) -> Placement {
    place(r, &[item.to_owned()], budget, By { actor, at: 9 }).unwrap()
}

#[test]
fn an_add_splits_its_subtree_once_free_leaves_pass_the_budget() {
    let mut r = two_groups(BUDGET, 0);
    assert!(place_one(&mut r, "g1-00", Some(BUDGET), USER.actor).is_empty(), "at the budget");
    add(&mut r, "late", Some("g1"));
    let placed = place_one(&mut r, "late", Some(BUDGET), USER.actor);
    assert_eq!(members(&placed.drafts), vec![("p".to_owned(), BUDGET + 1)]);
    assert_eq!(plan(&model(&r), None), Vec::<Draft>::new(), "written");
    // An import splits what it touches, however little.
    let mut r = two_groups(2, 0);
    assert_eq!(place_one(&mut r, "g1-00", None, USER.actor).drafts.len(), 1);
}

#[test]
fn new_work_joins_its_nearest_threaded_ancestor_as_its_actor_may() {
    let user = By { actor: USER.actor, at: 1 };
    let mut r = two_groups(1, 0);
    for (thread, approved, pm) in [("ok", true, false), ("loose", false, false), ("pm", true, true)]
    {
        let key = Key::new(kind::THREAD, thread).unwrap();
        write(&mut r, &key, thread::APPROVED, json!(approved));
        write(&mut r, &key, thread::PM_FLAG, json!(pm));
    }
    crate::model::start_run(&mut r, "boss", "pm", user).unwrap();
    write(&mut r, &item_key("g1").unwrap(), field::THREAD, json!("ok"));
    // The user and a project-management run join approved scope; any other agent proposes.
    for (leaf, actor, proposed) in
        [("u", "actor/user", false), ("m", "run/boss", false), ("a", "session/mac", true)]
    {
        add(&mut r, leaf, Some("g1-00"));
        let placed = place_one(&mut r, leaf, Some(BUDGET), actor);
        let [joined] = placed.inherited.as_slice() else { panic!("{actor}: {placed:?}") };
        assert_eq!(
            (joined.thread.as_str(), joined.proposal.is_some()),
            ("ok", proposed),
            "{actor}"
        );
    }
    // Unapproved scope takes anyone's work; a thread an item already has is never changed.
    write(&mut r, &item_key("g1").unwrap(), field::THREAD, json!("loose"));
    add(&mut r, "b", Some("g1-00"));
    let placed = place_one(&mut r, "b", Some(BUDGET), "session/mac");
    assert_eq!(placed.inherited[0].proposal, None);
    add(&mut r, "c", Some("g1-00"));
    write(&mut r, &item_key("c").unwrap(), field::THREAD, json!("own"));
    assert!(place_one(&mut r, "c", Some(BUDGET), "session/mac").is_empty());
    assert_eq!(model(&r).thread_of("c"), Some("own"));
}

#[test]
fn an_item_gaining_its_first_child_splits_its_subtree() {
    let mut r = two_groups(2, 0);
    add(&mut r, "kid", Some("g1-00"));
    let placed = place_one(&mut r, "kid", Some(BUDGET), USER.actor);
    assert_eq!(placed.drafts.len(), 1, "well under the budget, `g1-00` became a group");
    assert_eq!(placed.drafts[0].members, BTreeSet::from(["g1-01".to_owned(), "kid".to_owned()]));
    add(&mut r, "kid2", Some("g1-00"));
    assert!(place_one(&mut r, "kid2", Some(BUDGET), USER.actor).is_empty(), "a second child isn't");
}

#[test]
fn an_item_a_proposal_would_move_waits_for_the_user() {
    let mut r = two_groups(2, 0);
    let ok = Key::new(kind::THREAD, "ok").unwrap();
    write(&mut r, &ok, thread::APPROVED, json!(true));
    let agent = By { actor: "session/mac", at: 2 };
    let change = ScopeEdit::Member { item: "g1-00".into(), thread: Some("ok".into()) };
    let scope::Outcome::Proposed(id) = scope::edit(&mut r, &change, agent).unwrap() else {
        panic!("an agent proposes into approved scope")
    };
    let held = held(r.state()).unwrap();
    assert_eq!(held, BTreeSet::from(["g1-00".to_owned()]));
    let drafts = plan_with(&model(&r), None, &held);
    assert_eq!(drafts[0].members, BTreeSet::from(["g1-01".to_owned()]));
    assert_eq!(apply(&mut r, None, USER).unwrap(), drafts, "a split writes the same");
    scope::drop(&mut r, &id, By { actor: USER.actor, at: 3 }).unwrap();
    let freed = apply(&mut r, None, USER).unwrap();
    assert_eq!(freed[0].members, BTreeSet::from(["g1-00".to_owned()]), "dropped, it is free again");
}
