use super::*;
use crate::model::{By, add_item, item_key, put, set};
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
            claim(&mut r, missing, "session/s", 10, (LEASE, LEASE), false),
            Err(Error::NoThread(missing.into()).into())
        );
    }
    claim(&mut r, "t", "session/s", 10, (LEASE, LEASE), false).unwrap();
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
    claim(&mut r, "t", "session/new", 20, (LEASE, LEASE), false).unwrap();
    // The old holder keeps writing, yet its item claim is never renewed.
    set(&mut r, &item_key("x").unwrap(), field::NOTES, Some(&"busy"), by("session/old", 900))
        .unwrap();
    assert_eq!(renew_leases(&mut r, 9_900, LEASE, LEASE).unwrap(), []);
}

#[test]
fn a_resource_older_versions_stored_on_items_holds_nothing() {
    let mut r = replica("a");
    thread(&mut r, "one", &["x"]);
    thread(&mut r, "two", &["y"]);
    for id in ["x", "y"] {
        let key = item_key(id).unwrap();
        r.write(&key, "resource/gpu", "actor/user", 1, Some(serde_json::json!(true))).unwrap();
    }
    claim(&mut r, "one", "session/one", 10, (LEASE, LEASE), false).unwrap();
    claim(&mut r, "two", "session/two", 20, (LEASE, LEASE), false).unwrap();
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
    claim(&mut r, "t", "session/s", 30, (LEASE, LEASE), false).unwrap();
    assert_eq!(holder(&r, "t", 30), Some("session/s".into()));
}

#[test]
fn renew_leases_extend_only_active_winners_to_the_default_lease() {
    let (mut a, mut b) = (replica("a"), replica("b"));
    thread(&mut a, "busy", &["x"]);
    thread(&mut a, "idle", &["y"]);
    b.join(&a.state).unwrap();
    thread(&mut b, "theirs", &["z"]);
    claim(&mut a, "busy", "session/busy", 0, (LEASE / 5, LEASE), false).unwrap();
    claim(&mut a, "idle", "session/idle", 0, (LEASE, LEASE), false).unwrap();
    claim(&mut b, "theirs", "session/busy", 0, (LEASE, LEASE), false).unwrap();
    a.join(&b.state).unwrap();
    // Only the busy session writes anything else.
    set(&mut a, &item_key("x").unwrap(), field::NOTES, Some(&"progress"), by("session/busy", 50))
        .unwrap();

    let busy = thread_key("busy").unwrap();
    let renewed = renew_leases(&mut a, 75, LEASE, LEASE).unwrap();
    assert_eq!(renewed, vec![busy.clone()], "not the idle one, nor another replica's");
    let claim = winner(&a.state, &busy, 75).unwrap().unwrap();
    assert_eq!((claim.lease_start, claim.lease_expires), (0, Some(75 + LEASE)));
    assert!(
        winner(&a.state, &thread_key("idle").unwrap(), 1_000).unwrap().is_none(),
        "expired after one lease"
    );
    // Even the short initial lease renews to the default duration. It isn't due again at exactly half.
    assert_eq!(renew_leases(&mut a, 575, LEASE, LEASE).unwrap(), Vec::<Key>::new());
    assert_eq!(renew_leases(&mut a, 576, LEASE, LEASE).unwrap(), vec![busy.clone()]);
    assert_eq!(winner(&a.state, &busy, 576).unwrap().unwrap().lease_expires, Some(576 + LEASE));
    // Renewal writes don't keep a holder active once its last ordinary write leaves the window.
    assert_eq!(renew_leases(&mut a, 1_100, LEASE, LEASE).unwrap(), Vec::<Key>::new());
    // A renewed claim is still its holder's to release, though the replica wrote it.
    assert!(release(&mut a, &busy, "session/busy"));
    assert!(winner(&a.state, &busy, 1_100).unwrap().is_none());
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
        claim(&mut a, "t", "session/one", 10, (LEASE, LEASE), false).unwrap();
        claim(&mut b, "t", "session/two", 20, (LEASE, LEASE), false).unwrap();
        a.join(&b.state).unwrap();
        b.join(&a.state).unwrap();
        assert_eq!(holders(&a), ["session/one", "session/two"]);
        (a, b, c)
    };
    // The loser steps aside while the winner renews: both drop the loser's claim.
    let (mut a, mut b, _) = contest();
    assert!(release(&mut b, &t, "session/two"));
    assert!(!release(&mut b, &t, "session/two"), "nothing left to release");
    claim(&mut a, "t", "session/one", 30, (LEASE, LEASE), false).unwrap();
    a.join(&b.state).unwrap();
    b.join(&a.state).unwrap();
    assert_eq!(
        (holders(&a), holders(&b)),
        (vec!["session/one".to_owned()], vec!["session/one".to_owned()])
    );
    // A claim the release never saw survives it.
    let (_, mut b, mut c) = contest();
    claim(&mut c, "t", "session/three", 40, (LEASE, LEASE), false).unwrap();
    assert!(release(&mut b, &t, "session/two"));
    b.join(&c.state).unwrap();
    assert_eq!(holders(&b), ["session/one", "session/three"]);
    // The winner stepping aside leaves the thread to the loser, on every replica.
    let (mut a, b, _) = contest();
    assert!(release(&mut a, &t, "session/one"));
    a.join(&b.state).unwrap();
    assert_eq!(holder(&a, "t", 50).as_deref(), Some("session/two"));
}
