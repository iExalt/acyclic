use std::collections::BTreeMap;

use hegel::TestCase;
use hegel::generators as gs;
use serde_json::json;

use super::*;
use crate::claim;

const DAY: Millis = 24 * 60 * 60 * 1000;

fn id(name: &str) -> ReplicaId {
    ReplicaId::new(name).unwrap()
}

fn key(kind: &str, name: &str) -> Key {
    Key::new(kind, name).unwrap()
}

fn replica(name: &str) -> Replica {
    Replica::open(id(name), State::new())
}

fn values(state: &State, key: &Key, field: &str) -> Vec<Json> {
    state.field(key, field).into_iter().flatten().map(|(_, v)| v.value.clone()).collect()
}

fn op(run: &str, seq: u64, key: &Key, field: &str, value: Option<Json>, seen: Vec<Dot>) -> Op {
    Op { run: run.to_owned(), seq, key: key.clone(), field: field.to_owned(), value, seen }
}

fn round_trip(state: &State) -> State {
    State::from_files(&state.to_files()).unwrap()
}

#[test]
fn concurrent_writes_conflict_and_a_later_write_supersedes_them() {
    let item = key("item", "a1");
    let (mut a, mut b) = (replica("a"), replica("b"));
    a.write(&item, "title", "user", 1, Some(json!("from a"))).unwrap();
    b.write(&item, "title", "user", 1, Some(json!("from b"))).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(values(&a.state, &item, "title"), vec![json!("from a"), json!("from b")]);
    a.write(&item, "title", "user", 2, Some(json!("resolved"))).unwrap();
    b.join(&a.state).unwrap();
    assert_eq!(values(&b.state, &item, "title"), vec![json!("resolved")]);
}

#[test]
fn a_clear_survives_a_join_with_the_old_value() {
    let item = key("item", "a1");
    let (mut a, mut b) = (replica("a"), replica("b"));
    a.write(&item, "title", "user", 1, Some(json!("x"))).unwrap();
    b.join(&a.state).unwrap();
    a.write(&item, "title", "user", 2, None).unwrap();
    assert!(a.state.entity(&item).is_none(), "an entity with no values is absent");
    a.join(&b.state).unwrap();
    assert!(a.state.field(&item, "title").is_none());
    b.join(&a.state).unwrap();
    assert!(b.state.field(&item, "title").is_none());
}

#[test]
fn unknown_kinds_and_fields_pass_through() {
    let widget = key("widget", "w1");
    let (mut a, mut b) = (replica("a"), replica("b"));
    a.write(&widget, "x-future", "user", 1, Some(json!({"nested": [1, 2]}))).unwrap();
    b.join(&round_trip(&a.state)).unwrap();
    assert_eq!(values(&round_trip(&b.state), &widget, "x-future"), vec![json!({"nested": [1, 2]})]);
}

#[test]
fn files_are_canonical_whatever_the_key_order() {
    let item = key("item", "a1");
    let mut forward = serde_json::Map::new();
    let mut backward = serde_json::Map::new();
    for name in ["alpha", "beta", "gamma"] {
        forward.insert(name.to_owned(), json!(1));
    }
    for name in ["gamma", "beta", "alpha"] {
        backward.insert(name.to_owned(), json!(1));
    }
    let (mut a, mut b) = (replica("a"), replica("a"));
    a.write(&item, "f", "user", 1, Some(Json::Object(forward))).unwrap();
    b.write(&item, "f", "user", 1, Some(Json::Object(backward))).unwrap();
    assert_eq!(a.state.to_files(), b.state.to_files());
    let text = &a.state.to_files()["item/a1/a1.json"];
    assert!(text.find("alpha").unwrap() < text.find("gamma").unwrap());
}

#[test]
fn files_fan_out_by_id_and_reject_unsafe_names() {
    assert_eq!(key("item", "x").path(), "item/x_/x.json");
    assert_eq!(key("item", "abc").path(), "item/ab/abc.json");
    for bad in ["", "../x", ".hidden", "a/b", "a b", "Upper"] {
        assert!(Key::new("item", bad).is_err(), "{bad:?}");
    }
    assert!(ReplicaId::new("mac_1").is_err());
    assert!(ReplicaId::new("Mac").is_err());
}

#[test]
fn reading_refuses_newer_formats_and_bad_files() {
    let mut a = replica("a");
    a.write(&key("item", "a1"), "title", "user", 1, Some(json!("t"))).unwrap();
    let files = a.state.to_files();

    let mut newer = files.clone();
    newer.insert(
        HEADER_PATH.to_owned(),
        files[HEADER_PATH].replace("\"format_version\": 1", "\"format_version\": 2"),
    );
    assert!(matches!(State::from_files(&newer), Err(Error::Format(m)) if m.contains("version 2")));

    let mut moved = files.clone();
    let text = moved.remove("item/a1/a1.json").unwrap();
    moved.insert("item/zz/a1.json".to_owned(), text);
    assert!(State::from_files(&moved).is_err());

    let mut ahead = files;
    ahead.insert(
        HEADER_PATH.to_owned(),
        "{\"format_version\":1,\"context\":{\"seen\":{},\"clocks\":{}}}".to_owned(),
    );
    assert!(State::from_files(&ahead).is_err(), "a dot beyond the context");
}

#[test]
fn values_hold_integers_but_no_other_numbers() {
    let mut a = replica("a");
    let item = key("item", "a1");
    assert_eq!(a.write(&item, "size", "user", 1, Some(json!(1.5))), Err(Error::InvalidValue));
    assert_eq!(
        a.write(&item, "size", "user", 1, Some(json!({"deep": [2.0]}))),
        Err(Error::InvalidValue)
    );
    assert!(a.write(&item, "size", "user", 1, Some(json!(-3))).is_ok());
}

#[test]
fn the_clock_guard_allows_idle_time_and_refuses_backward_steps() {
    let mut a = replica("a");
    let item = key("item", "a1");
    let start = 1_000 * DAY;
    a.write(&item, "f", "user", start, Some(json!(1))).unwrap();
    assert_eq!(a.stamp(start + 3 * DAY, false), Ok(start + 3 * DAY), "three idle days pass");
    assert_eq!(a.stamp(start - 60_000, false), Ok(start), "a small step back is clamped");
    assert_eq!(
        a.stamp(start - CLOCK_SKEW_MS - 1, false),
        Err(Error::ClockJump { now: start - CLOCK_SKEW_MS - 1, last: start })
    );
    assert_eq!(a.stamp(start - DAY, true), Ok(start), "the override clamps instead");
}

#[test]
fn a_rebuilt_replica_never_reuses_its_dots() {
    let item = key("item", "a1");
    let mut a = replica("a");
    for n in 0..3 {
        a.write(&item, "f", "user", n, Some(json!(n))).unwrap();
    }
    let mut peer = replica("b");
    peer.join(&a.state).unwrap();

    let mut rebuilt = Replica::rebuilt(id("a"), State::new());
    assert_eq!(
        rebuilt.write(&item, "f", "user", 9, Some(json!(9))),
        Err(Error::NotContinuous(id("a")))
    );
    rebuilt.join(&peer.state).unwrap();
    assert_eq!(
        rebuilt.write(&item, "f", "user", 9, Some(json!(9))),
        Err(Error::NotContinuous(id("a"))),
        "a peer's entry alone doesn't restore it"
    );
    // Only after joining every state ref from the remote may the caller restore it.
    rebuilt.mark_restored();
    let dot = rebuilt.write(&item, "f", "user", 9, Some(json!(9))).unwrap();
    assert_eq!(dot, Some(Dot(id("a"), 4)));
}

#[test]
fn a_join_refuses_a_state_that_saw_more_of_my_writes() {
    // A stale rebuild, or a second clone writing under the same id: each would reuse dots.
    let item = key("item", "a1");
    let mut a = replica("a");
    a.write(&item, "f", "user", 1, Some(json!(1))).unwrap();
    let mut stale = replica("p1");
    stale.join(&a.state).unwrap();
    for n in 2..=4 {
        a.write(&item, "f", "user", n, Some(json!(n))).unwrap();
    }
    let mut fresh = replica("p2");
    fresh.join(&a.state).unwrap();

    let mut rebuilt = Replica::rebuilt(id("a"), State::new());
    rebuilt.join(&stale.state).unwrap();
    rebuilt.mark_restored(); // wrongly, from a stale peer
    rebuilt.write(&key("item", "new"), "f", "user", 5, Some(json!("new"))).unwrap();
    assert_eq!(
        rebuilt.join(&fresh.state),
        Err(Error::IdReused { replica: id("a"), mine: 2, theirs: 4 })
    );
}

#[test]
fn an_intact_replica_writes_offline() {
    let mut a = replica("a");
    let dot = a.write(&key("item", "a1"), "f", "user", 1, Some(json!(1))).unwrap();
    assert_eq!(dot, Some(Dot(id("a"), 1)));
}

#[test]
fn a_stale_run_conflicts_instead_of_losing_an_edit() {
    let item = key("item", "a1");
    let mut server = replica("server");
    let base = server.write(&item, "title", "user", 1, Some(json!("v1"))).unwrap().unwrap();
    // The user edits after the run read its base.
    server.write(&item, "title", "user", 2, Some(json!("user edit"))).unwrap();
    let applied =
        server.apply(&op("r1", 1, &item, "title", Some(json!("run edit")), vec![base]), 3).unwrap();
    assert!(matches!(applied, Applied::Applied(_)));
    assert_eq!(values(&server.state, &item, "title"), vec![json!("user edit"), json!("run edit")]);
}

#[test]
fn a_run_that_saw_the_current_values_replaces_them() {
    let item = key("item", "a1");
    let mut server = replica("server");
    let x = server.write(&item, "title", "user", 1, Some(json!("x"))).unwrap().unwrap();
    let mut mac = replica("mac");
    let y = mac.write(&item, "title", "user", 1, Some(json!("y"))).unwrap().unwrap();
    server.join(&mac.state).unwrap();
    server.apply(&op("r1", 1, &item, "title", Some(json!("merged")), vec![x, y]), 2).unwrap();
    assert_eq!(values(&server.state, &item, "title"), vec![json!("merged")]);
}

#[test]
fn a_run_supersedes_itself_and_a_resent_op_applies_once() {
    let item = key("item", "a1");
    let mut server = replica("server");
    let first = op("r1", 1, &item, "title", Some(json!("one")), vec![]);
    server.apply(&first, 1).unwrap();
    let after_first = server.state.clone();
    assert_eq!(server.apply(&first, 2), Ok(Applied::Duplicate));
    assert_eq!(server.state, after_first);
    server.apply(&op("r1", 2, &item, "title", Some(json!("two")), vec![]), 3).unwrap();
    assert_eq!(values(&server.state, &item, "title"), vec![json!("two")]);
}

#[test]
fn a_gap_or_another_replica_is_refused() {
    let item = key("item", "a1");
    let mut server = replica("server");
    server.apply(&op("r1", 1, &item, "f", Some(json!(1)), vec![]), 1).unwrap();
    assert_eq!(
        server.apply(&op("r1", 3, &item, "f", Some(json!(3)), vec![]), 2),
        Err(Error::Gap { run: "r1".to_owned(), expected: 2, got: 3 })
    );
    let mut other = replica("other");
    other.join(&server.state).unwrap();
    assert_eq!(
        other.apply(&op("r1", 2, &item, "f", Some(json!(2)), vec![]), 3),
        Err(Error::WrongReplica { run: "r1".to_owned(), applied_by: "server".to_owned() })
    );
}

#[test]
fn context_size_does_not_grow_with_runs() {
    let size = |runs: usize| {
        let mut server = replica("server");
        let mut mac = replica("mac");
        mac.write(&key("item", "m"), "f", "user", 1, Some(json!(1))).unwrap();
        server.join(&mac.state).unwrap();
        for run in 0..runs {
            let item = key("item", &format!("i{}", run % 10));
            server
                .apply(&op(&format!("r{run}"), 1, &item, "f", Some(json!(run)), vec![]), 2)
                .unwrap();
        }
        (server.state.context.seen.len(), server.state.context.clocks.len())
    };
    assert_eq!(size(1), size(200));
    assert_eq!(size(200), (2, 2));
}

#[test]
fn concurrent_claims_contest_and_a_seen_claim_is_not_stolen() {
    let item = key("item", "a1");
    let (mut mac, mut server) = (replica("mac"), replica("server"));
    claim::claim(&mut mac, &item, "session/mac", 100, Some(60_000), 600_000).unwrap();
    claim::claim(&mut server, &item, "run/r1", 90, None, 600_000).unwrap();

    let mut one = mac.state.clone();
    one.join(&server.state);
    let mut two = server.state.clone();
    two.join(&mac.state);
    assert_eq!(claim::live(&one, &item, 120).unwrap().len(), 2, "a contest");
    for now in [120, 200_000] {
        assert_eq!(
            claim::winner(&one, &item, now).unwrap(),
            claim::winner(&two, &item, now).unwrap()
        );
    }
    assert_eq!(
        claim::winner(&one, &item, 120).unwrap().unwrap().holder,
        "run/r1",
        "earliest lease wins"
    );

    mac.join(&server.state).unwrap();
    assert_eq!(
        claim::claim(&mut mac, &item, "session/mac", 130, Some(60_000), 600_000),
        Err(Error::Claimed(item.clone()))
    );
}

#[test]
fn a_contest_winner_renews_and_keeps_winning() {
    let item = key("item", "a1");
    let (mut a, mut b) = (replica("a"), replica("b"));
    claim::claim(&mut a, &item, "ha", 0, Some(100), 1_000).unwrap();
    claim::claim(&mut b, &item, "hb", 5, Some(100), 1_000).unwrap();
    a.join(&b.state).unwrap();
    b.join(&a.state).unwrap();
    assert_eq!(claim::winner(&a.state, &item, 10).unwrap().unwrap().holder, "ha");

    claim::claim(&mut a, &item, "ha", 50, Some(100), 1_000).unwrap();
    let renewed = claim::winner(&a.state, &item, 120).unwrap().unwrap();
    assert_eq!(
        (renewed.holder.as_str(), renewed.lease_start, renewed.lease_expires),
        ("ha", 0, Some(150))
    );
    assert_eq!(
        claim::claim(&mut b, &item, "hb", 60, Some(100), 1_000),
        Err(Error::Claimed(item.clone()))
    );
}

#[test]
fn an_unreadable_claim_is_not_hidden() {
    let item = key("item", "a1");
    let mut a = replica("a");
    a.write(&item, claim::FIELD, "future", 1, Some(json!({"holder": "x", "new-field": 1})))
        .unwrap();
    assert!(claim::live(&a.state, &item, 2).is_err());
    assert!(claim::claim(&mut a, &item, "me", 2, None, 10).is_err());
}

#[test]
fn a_write_never_goes_back_in_time() {
    let item = key("item", "a1");
    let mut a = replica("a");
    a.write(&item, "f", "user", 100, Some(json!(1))).unwrap();
    a.write(&item, "f", "user", 40, Some(json!(2))).unwrap();
    let register = a.state.field(&item, "f").unwrap();
    assert_eq!(register.values().next().unwrap().at, 100);
}

#[test]
fn reading_refuses_floats_and_non_canonical_files() {
    let mut a = replica("a");
    a.write(&key("item", "a1"), "f", "user", 1, Some(json!(1))).unwrap();
    let files = a.state.to_files();
    assert_eq!(State::from_files(&files).unwrap(), a.state);
    let path = "item/a1/a1.json";
    let mut float = files.clone();
    float.insert(path.to_owned(), files[path].replace("\"value\": 1", "\"value\": 1.5"));
    assert!(State::from_files(&float).is_err());
    let mut compact = files.clone();
    let value: Json = serde_json::from_str(&files[path]).unwrap();
    compact.insert(path.to_owned(), serde_json::to_string(&value).unwrap());
    assert!(
        matches!(State::from_files(&compact), Err(Error::Format(m)) if m.contains("canonical"))
    );
}

#[test]
fn leases_are_capped_and_expire() {
    let item = key("item", "a1");
    let mut mac = replica("mac");
    claim::claim(&mut mac, &item, "session/mac", 0, Some(DAY), 1_000).unwrap();
    let held = claim::winner(&mac.state, &item, 0).unwrap().unwrap();
    assert_eq!(held.lease_expires, Some(1_000));
    assert!(claim::winner(&mac.state, &item, 1_000).unwrap().is_none());
    assert!(held.starts_in_future(-CLOCK_SKEW_MS - 1));
    assert!(!held.starts_in_future(0));
}

#[test]
fn thread_and_global_fields_join_like_any_other() {
    use crate::schema::{GLOBAL, kind, settings, thread};
    let t = key(kind::THREAD, "t1");
    let global = key(kind::SETTINGS, GLOBAL);
    let (mut a, mut b) = (replica("a"), replica("b"));
    for field in [thread::PM_FLAG, thread::AUTONOMY, thread::LAUNCH, thread::PLACEMENT] {
        a.write(&t, field, "user", 1, Some(json!(field))).unwrap();
    }
    b.write(&global, settings::AUTONOMY, "user", 1, Some(json!("blockers"))).unwrap();
    a.join(&b.state).unwrap();
    let a = round_trip(&a.state);
    assert_eq!(a.entity(&t).unwrap().len(), 4);
    assert_eq!(values(&a, &global, settings::AUTONOMY), vec![json!("blockers")]);
}

// Property tests: probe 1's convergence properties.

const REPLICAS: [&str; 3] = ["r0", "r1", "r2"];

#[derive(Debug, Clone)]
enum Step {
    Write { replica: usize, entity: usize, field: usize, value: Option<i64> },
    Join { into: usize, from: usize },
    RunOp { run: usize, entity: usize, field: usize, value: Option<i64>, seen_mask: u8 },
}

fn below(tc: &TestCase, n: usize) -> usize {
    tc.draw(gs::integers::<usize>().max_value(n - 1))
}

fn value(tc: &TestCase) -> Option<i64> {
    // About one write in seven clears the field.
    (below(tc, 7) != 0).then(|| tc.draw(gs::integers::<i64>().min_value(0).max_value(4)))
}

/// Up to 40 random steps, weighted 3:2:2 toward writes over joins and run operations.
fn steps(tc: &TestCase) -> Vec<Step> {
    let count = below(tc, 41);
    (0..count)
        .map(|_| match below(tc, 7) {
            0..=2 => Step::Write {
                replica: below(tc, 3),
                entity: below(tc, 4),
                field: below(tc, 3),
                value: value(tc),
            },
            3 | 4 => Step::Join { into: below(tc, 3), from: below(tc, 3) },
            _ => Step::RunOp {
                run: below(tc, 4),
                entity: below(tc, 4),
                field: below(tc, 3),
                value: value(tc),
                seen_mask: tc.draw(gs::integers::<u8>()),
            },
        })
        .collect()
}

/// Runs the steps; each run is applied by replica `run % 3`, with a random subset of the field's current dots as
/// what it saw, so many runs are stale.
fn play(steps: &[Step]) -> Vec<Replica> {
    let mut replicas: Vec<Replica> = REPLICAS.iter().map(|name| replica(name)).collect();
    let mut seqs = [0u64; 4];
    for (at, step) in steps.iter().enumerate() {
        let at = Millis::try_from(at).unwrap();
        match step {
            Step::Write { replica, entity, field, value } => {
                let item = key("item", &format!("e{entity}"));
                replicas[*replica]
                    .write(&item, &format!("f{field}"), "user", at, value.map(Json::from))
                    .unwrap();
            },
            Step::Join { into, from } => {
                let other = replicas[*from].state.clone();
                replicas[*into].join(&other).unwrap();
            },
            Step::RunOp { run, entity, field, value, seen_mask } => {
                let applier = &mut replicas[run % 3];
                let item = key("item", &format!("e{entity}"));
                let field = format!("f{field}");
                let seen = applier
                    .state
                    .field(&item, &field)
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .filter(|(i, _)| seen_mask & (1 << (i % 8)) != 0)
                    .map(|(_, (dot, _))| dot.clone())
                    .collect();
                seqs[*run] += 1;
                let op = op(
                    &format!("run{run}"),
                    seqs[*run],
                    &item,
                    &field,
                    value.map(Json::from),
                    seen,
                );
                assert!(matches!(applier.apply(&op, at), Ok(Applied::Applied(_))));
            },
        }
    }
    replicas
}

fn joined(states: &[&State]) -> State {
    let mut result = State::new();
    for state in states {
        result.join(state);
    }
    result
}

/// P1: random writes and stale run operations across three replicas converge in any join order.
#[hegel::test(test_cases = 256)]
fn replicas_converge_in_any_join_order(tc: TestCase) {
    let replicas = play(&steps(&tc));
    let order = tc.draw(gs::permutations(vec![0usize, 1, 2]));
    for replica in &replicas {
        assert!(replica.state.check().is_ok());
    }
    let reference = joined(&[&replicas[0].state, &replicas[1].state, &replicas[2].state]);
    let shuffled = joined(&order.iter().map(|&i| &replicas[i].state).collect::<Vec<_>>());
    assert_eq!(shuffled, reference);
    // Gossip in the drawn order, twice round, leaves every replica identical.
    let mut states: Vec<State> = replicas.iter().map(|r| r.state.clone()).collect();
    for _ in 0..2 {
        for &i in &order {
            for j in 0..3 {
                let other = states[j].clone();
                states[i].join(&other);
            }
        }
    }
    for state in &states {
        assert_eq!(state, &reference);
    }
    assert_eq!(round_trip(&reference), reference);
}

/// P2: the join is commutative, associative and idempotent.
#[hegel::test(test_cases = 256)]
fn join_is_a_semilattice(tc: TestCase) {
    let replicas = play(&steps(&tc));
    let (a, b, c) = (&replicas[0].state, &replicas[1].state, &replicas[2].state);
    assert_eq!(joined(&[a, b]), joined(&[b, a]));
    assert_eq!(joined(&[&joined(&[a, b]), c]), joined(&[a, &joined(&[b, c])]));
    assert_eq!(&joined(&[a, a]), a);
}

/// P6: concurrent claims from two replicas resolve to the same winner, for the same `now`, in either join order.
#[hegel::test(test_cases = 256)]
fn claim_winners_agree(tc: TestCase) {
    let start = || tc.draw(gs::integers::<i64>().min_value(0).max_value(999));
    let lease = || tc.draw(gs::optional(gs::integers::<i64>().min_value(1).max_value(1_999)));
    let item = key("item", "a1");
    let (mut x, mut y) = (replica("x"), replica("y"));
    claim::claim(&mut x, &item, "run/x", start(), lease(), 10_000).unwrap();
    claim::claim(&mut y, &item, "run/y", start(), lease(), 10_000).unwrap();
    let now = tc.draw(gs::integers::<i64>().min_value(0).max_value(2_999));
    let one = joined(&[&x.state, &y.state]);
    let two = joined(&[&y.state, &x.state]);
    assert_eq!(claim::winner(&one, &item, now).unwrap(), claim::winner(&two, &item, now).unwrap());
}

#[test]
fn canonical_files_are_stable_across_join_orders() {
    let steps: Vec<Step> = (0..30i64)
        .map(|n| {
            let i = usize::try_from(n).unwrap();
            Step::Write { replica: i % 3, entity: i % 4, field: i % 3, value: Some(n) }
        })
        .collect();
    let replicas = play(&steps);
    let a = joined(&[&replicas[0].state, &replicas[1].state, &replicas[2].state]).to_files();
    let b = joined(&[&replicas[2].state, &replicas[0].state, &replicas[1].state]).to_files();
    assert_eq!(a, b);
    let _: BTreeMap<String, String> = a;
}
