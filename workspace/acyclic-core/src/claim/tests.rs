use super::*;
use crate::model::SetField;
use crate::model::{By, ItemState, add_item, item_key, put, set, set_element, set_state};
use crate::schema::{item as field, thread as thread_field};
use crate::state::ReplicaId;

const LEASE: Millis = 1_000;

fn replica(name: &str) -> Replica {
    Replica::open(ReplicaId::new(name).unwrap(), State::new())
}

fn by(actor: &'static str, at: Millis) -> By<'static> {
    By { actor, at }
}

/// A thread record holding the given new items.
fn thread(r: &mut Replica, id: &str, members: &[&str]) {
    set(r, &thread_key(id).unwrap(), thread_field::TITLE, Some(&id), by("actor/user", 1)).unwrap();
    for member in members {
        add_item(r, member, member, by("actor/user", 1)).unwrap();
        put(r, &item_key(member).unwrap(), field::THREAD, Some(&id), by("actor/user", 1)).unwrap();
    }
}

fn holds(r: &mut Replica, id: &str, resource: &str) {
    set_element(r, &item_key(id).unwrap(), SetField::Resource, resource, true, by("actor/user", 1))
        .unwrap();
}

fn holder(r: &Replica, thread: &str, now: Millis) -> Option<String> {
    winner(&r.state, &thread_key(thread).unwrap(), now).unwrap().map(|c| c.holder)
}

#[test]
fn only_threads_with_a_record_are_claimed_and_their_claim_covers_every_member() {
    let mut r = replica("a");
    thread(&mut r, "t", &["x", "y"]);
    add_item(&mut r, "loose", "loose", by("actor/user", 1)).unwrap();
    put(&mut r, &item_key("loose").unwrap(), field::THREAD, Some(&"ghost"), by("actor/user", 1))
        .unwrap();
    for missing in ["ghost", "x"] {
        assert_eq!(
            claim(&mut r, missing, "session/s", 10, LEASE, LEASE),
            Err(Error::NoThread(missing.into()))
        );
    }
    claim(&mut r, "t", "session/s", 10, LEASE, LEASE).unwrap();
    let model = Model::read(&r.state).unwrap();
    for id in ["x", "y"] {
        let covering = covering(&r.state, &model, id, 20).unwrap();
        assert_eq!(covering.iter().map(|c| c.holder.as_str()).collect::<Vec<_>>(), ["session/s"]);
    }
    assert_eq!(covering(&r.state, &model, "loose", 20).unwrap(), []);
}

#[test]
fn claims_older_versions_kept_on_items_are_never_read() {
    let mut r = replica("a");
    thread(&mut r, "t", &["x"]);
    let old = Claim {
        holder: "session/old".into(),
        replica: "a".into(),
        lease_start: 0,
        lease_expires: Some(10_000),
    };
    r.write(
        &item_key("x").unwrap(),
        FIELD,
        "session/old",
        5,
        Some(serde_json::to_value(old).unwrap()),
    )
    .unwrap();
    let model = Model::read(&r.state).unwrap();
    assert_eq!(covering(&r.state, &model, "x", 20).unwrap(), []);
    claim(&mut r, "t", "session/new", 20, LEASE, LEASE).unwrap();
    // The old holder keeps writing, yet its item claim is never renewed.
    set(&mut r, &item_key("x").unwrap(), field::NOTES, Some(&"busy"), by("session/old", 900))
        .unwrap();
    assert_eq!(renew_leases(&mut r, 9_900, LEASE, LEASE).unwrap(), []);
}

#[test]
fn a_named_resource_is_held_by_one_thread_at_a_time_through_its_open_members() {
    let mut r = replica("a");
    thread(&mut r, "one", &["x", "x2"]);
    thread(&mut r, "two", &["y"]);
    thread(&mut r, "three", &["z"]);
    // Members of one thread share a resource freely.
    for id in ["x", "x2", "y"] {
        holds(&mut r, id, "gpu");
    }
    claim(&mut r, "one", "session/one", 10, LEASE, LEASE).unwrap();
    assert_eq!(
        claim(&mut r, "two", "session/two", 20, LEASE, LEASE),
        Err(Error::ResourceHeld {
            resource: "gpu".into(),
            thread: "one".into(),
            holder: "session/one".into()
        })
    );
    // The same holder may take both, and anyone may once the lease ends.
    claim(&mut r, "two", "session/one", 20, LEASE, LEASE).unwrap();
    claim(&mut r, "three", "session/two", 20, LEASE, LEASE).unwrap();
    let mut later = replica("b");
    later.join(&r.state).unwrap();
    claim(&mut later, "two", "session/two", 2_000, LEASE, LEASE).unwrap();
    // Finished work holds nothing: once one's members are done, its claim holds no resource.
    let mut done = replica("c");
    done.join(&r.state).unwrap();
    for id in ["x", "x2"] {
        set_state(&mut done, id, ItemState::Done, by("session/one", 30)).unwrap();
    }
    release(&mut done, &thread_key("two").unwrap(), "session/one");
    claim(&mut done, "two", "session/two", 40, LEASE, LEASE).unwrap();
}

#[test]
fn a_stored_claim_without_a_lease_holds_nothing() {
    // Older versions wrote claims without a lease for runs; such a claim must neither fail a read nor hold its thread.
    let mut r = replica("a");
    thread(&mut r, "t", &["x"]);
    let stored = serde_json::json!({
        "holder": "run/r1", "replica": "a", "lease_start": 10, "lease_expires": null
    });
    r.write(&thread_key("t").unwrap(), FIELD, "run/r1", 10, Some(stored)).unwrap();
    assert_eq!(holder(&r, "t", 20), None);
    assert_eq!(live(&r.state, &thread_key("t").unwrap(), 20).unwrap(), []);
    claim(&mut r, "t", "session/s", 30, LEASE, LEASE).unwrap();
    assert_eq!(holder(&r, "t", 30), Some("session/s".into()));
}

#[test]
fn sync_renews_only_the_leases_of_holders_still_at_work() {
    let (mut a, mut b) = (replica("a"), replica("b"));
    thread(&mut a, "busy", &["x"]);
    thread(&mut a, "idle", &["y"]);
    b.join(&a.state).unwrap();
    thread(&mut b, "theirs", &["z"]);
    claim(&mut a, "busy", "session/busy", 0, LEASE, LEASE).unwrap();
    claim(&mut a, "idle", "session/idle", 0, LEASE, LEASE).unwrap();
    claim(&mut b, "theirs", "session/busy", 0, LEASE, LEASE).unwrap();
    a.join(&b.state).unwrap();
    // Only the busy session writes anything else.
    set(&mut a, &item_key("x").unwrap(), field::NOTES, Some(&"progress"), by("session/busy", 600))
        .unwrap();

    assert!(
        renew_leases(&mut a, 400, LEASE, LEASE).unwrap().is_empty(),
        "over half the lease is left"
    );
    let busy = thread_key("busy").unwrap();
    let renewed = renew_leases(&mut a, 900, LEASE, LEASE).unwrap();
    assert_eq!(renewed, vec![busy.clone()], "not the idle one, nor another replica's");
    let claim = winner(&a.state, &busy, 1_500).unwrap().unwrap();
    assert_eq!((claim.lease_start, claim.lease_expires), (0, Some(1_900)));
    assert!(
        winner(&a.state, &thread_key("idle").unwrap(), 1_000).unwrap().is_none(),
        "expired after one lease"
    );
    // Renewed once a burst of syncs, then not again until half the new lease has passed.
    assert_eq!(renew_leases(&mut a, 950, LEASE, LEASE).unwrap(), Vec::<Key>::new());
    // Renewals are the replica's writes, so they don't keep a holder looking busy.
    assert_eq!(renew_leases(&mut a, 1_800, LEASE, LEASE).unwrap(), Vec::<Key>::new());
    // A renewed claim is still its holder's to release, though the replica wrote it.
    assert!(release(&mut a, &busy, "session/busy"));
    assert!(winner(&a.state, &busy, 1_500).unwrap().is_none());
}

#[test]
fn claims_made_apart_on_one_resource_are_a_clash() {
    let mut a = replica("a");
    thread(&mut a, "one", &["x"]);
    thread(&mut a, "two", &["y"]);
    for id in ["x", "y"] {
        holds(&mut a, id, "gpu");
    }
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    claim(&mut a, "one", "session/one", 10, LEASE, LEASE).unwrap();
    claim(&mut b, "two", "session/two", 10, LEASE, LEASE).unwrap();
    a.join(&b.state).unwrap();
    let model = Model::read(&a.state).unwrap();
    assert_eq!(
        clashes(&a.state, &model, 20).unwrap(),
        vec![Clash {
            resource: "gpu".into(),
            holders: vec![
                ("one".into(), "session/one".into()),
                ("two".into(), "session/two".into())
            ]
        }]
    );
    assert!(clashes(&a.state, &model, 2_000).unwrap().is_empty(), "gone when the leases end");
}

#[test]
fn a_release_drops_only_its_holders_claim_whatever_races_it() {
    let t = thread_key("t").unwrap();
    let holders = |r: &Replica| -> Vec<String> {
        live(&r.state, &t, 50).unwrap().into_iter().map(|c| c.holder).collect()
    };
    // A contest seen on both: one claimed first, so one wins.
    let contest = || {
        let mut a = replica("a");
        thread(&mut a, "t", &["x"]);
        let mut b = replica("b");
        b.join(&a.state).unwrap();
        let mut c = replica("c");
        c.join(&a.state).unwrap();
        claim(&mut a, "t", "session/one", 10, LEASE, LEASE).unwrap();
        claim(&mut b, "t", "session/two", 20, LEASE, LEASE).unwrap();
        a.join(&b.state).unwrap();
        b.join(&a.state).unwrap();
        assert_eq!(holders(&a), ["session/one", "session/two"]);
        (a, b, c)
    };
    // The loser steps aside while the winner renews: both drop the loser's claim.
    let (mut a, mut b, _) = contest();
    assert!(release(&mut b, &t, "session/two"));
    assert!(!release(&mut b, &t, "session/two"), "nothing left to release");
    claim(&mut a, "t", "session/one", 30, LEASE, LEASE).unwrap();
    a.join(&b.state).unwrap();
    b.join(&a.state).unwrap();
    assert_eq!(
        (holders(&a), holders(&b)),
        (vec!["session/one".to_owned()], vec!["session/one".to_owned()])
    );
    // A claim the release never saw survives it.
    let (_, mut b, mut c) = contest();
    claim(&mut c, "t", "session/three", 40, LEASE, LEASE).unwrap();
    assert!(release(&mut b, &t, "session/two"));
    b.join(&c.state).unwrap();
    assert_eq!(holders(&b), ["session/one", "session/three"]);
    // The winner stepping aside leaves the thread to the loser, on every replica.
    let (mut a, b, _) = contest();
    assert!(release(&mut a, &t, "session/one"));
    a.join(&b.state).unwrap();
    assert_eq!(holder(&a, "t", 50).as_deref(), Some("session/two"));
}
