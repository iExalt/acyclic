use serde_json::json;

use super::*;
use crate::model::{By, ItemState, Model, Value, add_item, item_key, set_parent, set_state};
use crate::schema::item as field;
use crate::state::{Replica, ReplicaId, State};

const USER: By<'static> = By { actor: "actor/user", at: 1 };

/// A parent's children, each as its thread and state, and the thread the parent itself holds.
struct Family {
    children: &'static [(Option<&'static str>, ItemState)],
    parent_thread: Option<&'static str>,
}

fn model_of(family: &Family) -> (Model, Vec<String>) {
    let mut r = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
    add_item(&mut r, "p", "P", USER).unwrap();
    let mut ids = Vec::new();
    for (n, (thread, state)) in family.children.iter().enumerate() {
        let id = format!("c{n}");
        add_item(&mut r, &id, &id, USER).unwrap();
        set_parent(&mut r, &id, Some("p"), USER).unwrap();
        if let Some(thread) = thread {
            let key = item_key(&id).unwrap();
            r.write(&key, field::THREAD, USER.actor, 1, Some(json!(thread))).unwrap();
        }
        set_state(&mut r, &id, *state, USER).unwrap();
        ids.push(id);
    }
    (Model::read(r.state()).unwrap(), ids)
}

#[test]
fn a_new_child_joins_the_thread_its_siblings_share_and_nothing_further_up() {
    use ItemState::{Backlog, Canceled, Done};
    let joins = |thread: &str| Placement::Joins(thread.to_owned());
    let cases: [(&str, Family, Placement); 9] = [
        (
            "all agree",
            Family { children: &[(Some("t"), Backlog), (Some("t"), Backlog)], parent_thread: None },
            joins("t"),
        ),
        (
            "disagree",
            Family { children: &[(Some("t"), Backlog), (Some("u"), Backlog)], parent_thread: None },
            Placement::Unclear,
        ),
        (
            "some unthreaded",
            Family { children: &[(Some("t"), Backlog), (None, Backlog)], parent_thread: None },
            Placement::Unclear,
        ),
        (
            "all unthreaded",
            Family { children: &[(None, Backlog), (None, Done)], parent_thread: Some("up") },
            Placement::Loose,
        ),
        (
            "only done siblings",
            Family { children: &[(Some("t"), Done), (Some("t"), Done)], parent_thread: None },
            joins("t"),
        ),
        (
            "only canceled siblings",
            Family { children: &[(Some("t"), Canceled)], parent_thread: None },
            joins("t"),
        ),
        (
            "done and open siblings that agree",
            Family {
                children: &[(Some("t"), Done), (Some("t"), Backlog)],
                parent_thread: Some("up"),
            },
            joins("t"),
        ),
        (
            "no siblings under a threaded parent",
            Family { children: &[], parent_thread: Some("up") },
            joins("up"),
        ),
        (
            "no siblings under an unthreaded parent",
            Family { children: &[], parent_thread: None },
            Placement::Loose,
        ),
    ];
    for (name, family, expected) in cases {
        let (model, ids) = model_of(&family);
        let siblings: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(placement(&model, &siblings, family.parent_thread), expected, "{name}");
    }
}

#[test]
fn siblings_in_conflict_leave_the_placement_to_the_caller() {
    let family = Family { children: &[(Some("t"), ItemState::Backlog)], parent_thread: None };
    let (mut model, ids) = model_of(&family);
    let item = model.items.get_mut("c0").unwrap();
    item.thread = Value::Conflict(vec![Some("t".into()), Some("u".into())]);
    // Alone, and beside one that reads alike.
    let one = [ids[0].as_str()];
    let twins = [ids[0].as_str(), ids[0].as_str()];
    assert_eq!(placement(&model, &one, None), Placement::Unclear);
    assert_eq!(placement(&model, &twins, None), Placement::Unclear);
}
