use super::*;
use crate::model::{
    By, RunState, add_item, item_key, set, set_element, set_parent, set_run_state, start_run,
};
use crate::schema::{item as field, set as sets};
use crate::state::ReplicaId;

const LEASE: Millis = 1_000;

fn replica(name: &str) -> Replica {
    Replica::open(ReplicaId::new(name).unwrap(), State::new())
}

fn by(actor: &'static str, at: Millis) -> By<'static> {
    By { actor, at }
}

fn items(r: &mut Replica, ids: &[&str]) {
    for id in ids {
        add_item(r, id, id, by("actor/user", 1)).unwrap();
    }
}

fn holds(r: &mut Replica, id: &str, resource: &str) {
    set_element(r, &item_key(id).unwrap(), sets::RESOURCE, resource, true, by("actor/user", 1))
        .unwrap();
}

fn holder(r: &Replica, id: &str, now: Millis) -> Option<String> {
    let model = Model::read(&r.state).unwrap();
    winner_with(&r.state, &item_key(id).unwrap(), now, &|h| model.run_alive(h))
        .unwrap()
        .map(|c| c.holder)
}

#[test]
fn only_leaves_are_claimed() {
    let mut r = replica("a");
    items(&mut r, &["goal", "leaf"]);
    set_parent(&mut r, "leaf", Some("goal"), by("actor/user", 2)).unwrap();
    let goal = item_key("goal").unwrap();
    assert_eq!(
        claim(&mut r, &goal, "session/s", 10, Some(LEASE), LEASE),
        Err(Error::NotALeaf(goal))
    );
    claim(&mut r, &item_key("leaf").unwrap(), "session/s", 10, Some(LEASE), LEASE).unwrap();
}

#[test]
fn a_named_resource_is_held_by_one_holder_at_a_time() {
    let mut r = replica("a");
    items(&mut r, &["x", "y", "z"]);
    for id in ["x", "y"] {
        holds(&mut r, id, "gpu");
    }
    claim(&mut r, &item_key("x").unwrap(), "session/one", 10, Some(LEASE), LEASE).unwrap();
    let y = item_key("y").unwrap();
    assert_eq!(
        claim(&mut r, &y, "session/two", 20, Some(LEASE), LEASE),
        Err(Error::ResourceHeld {
            resource: "gpu".into(),
            item: item_key("x").unwrap(),
            holder: "session/one".into()
        })
    );
    // The same holder may take both, and anyone may once the lease ends.
    claim(&mut r, &y, "session/one", 20, Some(LEASE), LEASE).unwrap();
    claim(&mut r, &item_key("z").unwrap(), "session/two", 20, Some(LEASE), LEASE).unwrap();
    let mut later = replica("b");
    later.join(&r.state).unwrap();
    claim(&mut later, &y, "session/two", 2_000, Some(LEASE), LEASE).unwrap();
    claim(&mut later, &item_key("x").unwrap(), "session/two", 2_000, Some(LEASE), LEASE).unwrap();
}

#[test]
fn a_runs_claim_lapses_when_its_run_ends() {
    let mut r = replica("a");
    items(&mut r, &["x"]);
    let x = item_key("x").unwrap();
    start_run(&mut r, "r1", "t1", by("actor/user", 2)).unwrap();
    claim(&mut r, &x, "run/r1", 10, None, LEASE).unwrap();
    assert_eq!(holder(&r, "x", 1_000_000), Some("run/r1".into()), "no lease: held while alive");
    set_run_state(&mut r, "r1", RunState::Stopped, by("actor/user", 20)).unwrap();
    assert_eq!(holder(&r, "x", 30), None);
    claim(&mut r, &x, "session/s", 30, Some(LEASE), LEASE).unwrap();
}

#[test]
fn sync_renews_only_the_leases_of_holders_still_at_work() {
    let (mut a, mut b) = (replica("a"), replica("b"));
    items(&mut a, &["busy", "idle"]);
    b.join(&a.state).unwrap();
    items(&mut b, &["theirs"]);
    claim(&mut a, &item_key("busy").unwrap(), "session/busy", 0, Some(LEASE), LEASE).unwrap();
    claim(&mut a, &item_key("idle").unwrap(), "session/idle", 0, Some(LEASE), LEASE).unwrap();
    claim(&mut b, &item_key("theirs").unwrap(), "session/busy", 0, Some(LEASE), LEASE).unwrap();
    a.join(&b.state).unwrap();
    // Only the busy session writes anything else.
    set(
        &mut a,
        &item_key("busy").unwrap(),
        field::NOTES,
        Some(&"progress"),
        by("session/busy", 600),
    )
    .unwrap();

    let renewed = renew_leases(&mut a, 900, LEASE, LEASE).unwrap();
    assert_eq!(renewed, vec![item_key("busy").unwrap()], "not the idle one, nor another replica's");
    let busy = winner(&a.state, &item_key("busy").unwrap(), 1_500).unwrap().unwrap();
    assert_eq!((busy.lease_start, busy.lease_expires), (0, Some(1_900)));
    assert!(
        winner(&a.state, &item_key("idle").unwrap(), 1_000).unwrap().is_none(),
        "expired after one lease"
    );
    // Renewals are the replica's writes, so they don't keep a holder looking busy.
    assert!(renew_leases(&mut a, 1_800, LEASE, LEASE).unwrap().is_empty());
}

#[test]
fn claims_made_apart_on_one_resource_are_a_clash() {
    let mut a = replica("a");
    items(&mut a, &["x", "y"]);
    for id in ["x", "y"] {
        holds(&mut a, id, "gpu");
    }
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    claim(&mut a, &item_key("x").unwrap(), "session/one", 10, Some(LEASE), LEASE).unwrap();
    claim(&mut b, &item_key("y").unwrap(), "session/two", 10, Some(LEASE), LEASE).unwrap();
    a.join(&b.state).unwrap();
    let model = Model::read(&a.state).unwrap();
    assert_eq!(
        clashes(&a.state, &model, 20).unwrap(),
        vec![Clash {
            resource: "gpu".into(),
            holders: vec![("x".into(), "session/one".into()), ("y".into(), "session/two".into())]
        }]
    );
    assert!(clashes(&a.state, &model, 2_000).unwrap().is_empty(), "gone when the leases end");
}
