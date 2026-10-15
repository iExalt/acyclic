use serde_json::json;

use super::*;
use crate::model::{By, ItemState, add_item, item_key, link, set_parent, set_state};
use crate::schema::{item as field, kind, thread};
use crate::state::{Key, Replica, ReplicaId, State};

const USER: By<'static> = By { actor: "actor/user", at: 1 };

fn write(r: &mut Replica, key: &Key, name: &str, value: serde_json::Value) {
    r.write(key, name, USER.actor, USER.at, Some(value)).unwrap();
}

/// Project `p` with children `a` (thread t1), `b` (thread t2, done) and `c` (no thread); `a` blocks on `b`, `c`
/// follows `a`, `a` relates to `z`, an archived top-level item; `x` is outside `p` and blocks `c`.
fn model() -> Model {
    let mut r = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
    for (id, title) in [
        ("p", "Project"),
        ("a", "Say \"hi\" <b>&amp;</b> | ; [x] {y}\nnext line"),
        ("b", "Ünïcödé # end"),
        ("c", "end"),
        ("x", "Outside"),
        ("z", "Old"),
    ] {
        add_item(&mut r, id, title, USER).unwrap();
    }
    for id in ["a", "b", "c"] {
        set_parent(&mut r, id, Some("p"), USER).unwrap();
    }
    for (id, title) in [("t1", "First \"thread\""), ("t2", "Second")] {
        write(&mut r, &Key::new(kind::THREAD, id).unwrap(), thread::TITLE, json!(title));
    }
    write(&mut r, &item_key("a").unwrap(), field::THREAD, json!("t1"));
    write(&mut r, &item_key("b").unwrap(), field::THREAD, json!("t2"));
    set_state(&mut r, "b", ItemState::Done, USER).unwrap();
    write(&mut r, &item_key("z").unwrap(), field::ARCHIVED, json!(true));
    link(&mut r, "a", "b", LinkKind::Blocks, USER).unwrap();
    link(&mut r, "c", "a", LinkKind::After, USER).unwrap();
    link(&mut r, "a", "z", LinkKind::Relates, USER).unwrap();
    link(&mut r, "c", "x", LinkKind::Blocks, USER).unwrap();
    Model::read(r.state()).unwrap()
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn a_scope_selects_items_and_draws_link_ends_outside_it_as_ghosts() {
    let m = model();
    let all = view(&m, &Scope::default()).unwrap();
    assert_eq!(all.items, set(&["a", "b", "c", "p", "x"]), "archived hidden");
    assert_eq!(all.ghosts, set(&["z"]), "an archived link end is a ghost");
    let with_archived = view(&m, &Scope { include_archived: true, ..Scope::default() }).unwrap();
    assert_eq!((with_archived.items.len(), with_archived.ghosts.len()), (6, 0));

    let root = view(&m, &Scope { root: Some("p"), ..Scope::default() }).unwrap();
    assert_eq!(root.items, set(&["a", "b", "c", "p"]));
    assert_eq!(root.ghosts, set(&["x", "z"]), "one hop out, never the root's parent");
    let both =
        view(&m, &Scope { root: Some("p"), thread: Some("t1"), ..Scope::default() }).unwrap();
    assert_eq!(both.items, set(&["a"]), "root and thread intersect");
    assert_eq!(both.ghosts, set(&["b", "c", "z"]));
    assert!(both.edges.iter().all(|e| e.kind != EdgeKind::Tree), "p isn't drawn, so no tree edge");

    assert_eq!(
        view(&m, &Scope { root: Some("nope"), ..Scope::default() }),
        Err(ScopeError::NoItem("nope".into()))
    );
    assert_eq!(
        view(&m, &Scope { thread: Some("nope"), ..Scope::default() }),
        Err(ScopeError::NoThread("nope".into()))
    );
}

#[test]
fn ids_are_injective_and_never_keywords() {
    assert_eq!(node_id('n', "end"), "n_end");
    assert_ne!(node_id('n', "a-b"), node_id('n', "a_b"));
    assert_ne!(node_id('n', "a.b"), node_id('n', "a_2e"));
    assert!(node_id('t', "x-1.y_z").bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
}

#[test]
fn each_scope_has_its_own_page_name() {
    let scope = |root, thread| scope_name(&Scope { root, thread, include_archived: false });
    assert_eq!(scope(None, None), "all");
    assert_eq!(scope(Some("a-1"), None), "root-a_2d1");
    assert_eq!(scope(None, Some("a-1")), "thread-a_2d1");
    assert_eq!(scope(Some("x"), Some("y")), "root-x-thread-y");
}

#[test]
fn dot_escapes_titles_and_clusters_threads() {
    let m = model();
    insta::assert_snapshot!(dot(&m, &view(&m, &Scope::default()).unwrap()));
}

#[test]
fn mermaid_escapes_titles_and_clusters_threads() {
    let m = model();
    insta::assert_snapshot!(mermaid(&m, &view(&m, &Scope::default()).unwrap()));
}
