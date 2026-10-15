use hegel::TestCase;
use hegel::generators as gs;

use super::*;
use crate::model::{add_item, start_run};
use crate::needs;
use crate::ready::{Ancestry, Readiness};
use crate::schema::thread;
use crate::state::{Millis, ReplicaId};

fn replica(name: &str) -> Replica {
    Replica::open(ReplicaId::new(name).unwrap(), State::new())
}

fn by(actor: &str, at: Millis) -> By<'_> {
    By { actor, at }
}

fn thread_key(id: &str) -> Key {
    Key::new(kind::THREAD, id).unwrap()
}

/// A tracker with items x and y, approved thread `ok` holding x, unapproved thread `draft`, a project-management
/// thread `pm` with run `boss`, and run `worker` on `ok`.
fn setup() -> Replica {
    let mut r = replica("base");
    let user = by(USER, 1);
    for id in ["x", "y"] {
        add_item(&mut r, id, id, user).unwrap();
    }
    for (id, approved, pm) in [("ok", true, false), ("draft", false, false), ("pm", true, true)] {
        model::set(&mut r, &thread_key(id), thread::APPROVED, Some(&approved), user).unwrap();
        model::set(&mut r, &thread_key(id), thread::PM_FLAG, Some(&pm), user).unwrap();
    }
    start_run(&mut r, "boss", "pm", user).unwrap();
    start_run(&mut r, "worker", "ok", user).unwrap();
    edit(&mut r, &member("x", Some("ok")), user).unwrap();
    r
}

fn member(item: &str, thread: Option<&str>) -> ScopeEdit {
    ScopeEdit::Member { item: item.to_owned(), thread: thread.map(str::to_owned) }
}

#[test]
fn only_the_declared_user_is_the_user() {
    let r = setup();
    let model = Model::read(&r.state).unwrap();
    assert_eq!(classify(&model, USER), Actor::User);
    assert_eq!(classify(&model, "run/boss"), Actor::ProjectManager);
    for agent in ["run/worker", "run/unknown", "session/mac", "", "user", "actor/user2"] {
        assert_eq!(classify(&model, agent), Actor::Agent, "{agent:?}");
    }
}

#[test]
fn an_agents_edit_to_approved_scope_waits_and_others_apply() {
    let mut r = setup();
    let agent = by("run/worker", 5);
    // Unapproved scope: applies.
    assert_eq!(edit(&mut r, &member("y", Some("draft")), agent).unwrap(), Outcome::Applied);
    // Into, out of, or across an approved thread: proposed.
    for change in [
        member("y", Some("ok")),
        member("x", None),
        member("x", Some("draft")),
        ScopeEdit::ItemRepository { item: "x".into(), repository: "code".into(), present: true },
        ScopeEdit::ThreadRepository {
            thread: "ok".into(),
            repository: "code".into(),
            present: true,
        },
    ] {
        assert!(
            matches!(edit(&mut r, &change, agent).unwrap(), Outcome::Proposed(_)),
            "{change:?}"
        );
    }
    let model = Model::read(&r.state).unwrap();
    assert_eq!(model.members("ok"), vec!["x"], "nothing applied");
    assert_eq!(proposals(&r.state).unwrap().len(), 5);
    // Asking again gets the pending proposal back; another agent's ask is its own.
    let first = proposals(&r.state).unwrap().into_keys().collect::<Vec<_>>();
    let again = edit(&mut r, &member("x", None), agent).unwrap();
    assert!(matches!(&again, Outcome::Proposed(id) if first.contains(id)), "{again:?}");
    let other = edit(&mut r, &member("x", None), by("session/mac", 5)).unwrap();
    assert!(matches!(&other, Outcome::Proposed(id) if !first.contains(id)), "{other:?}");
    assert_eq!(proposals(&r.state).unwrap().len(), 6);
    // The user and a project-management agent apply at once.
    assert_eq!(
        edit(&mut r, &member("y", Some("ok")), by("run/boss", 6)).unwrap(),
        Outcome::Applied
    );
    assert_eq!(edit(&mut r, &member("x", None), by(USER, 7)).unwrap(), Outcome::Applied);
    assert_eq!(Model::read(&r.state).unwrap().members("ok"), vec!["y"]);
}

#[test]
fn an_agent_cant_unapprove_its_way_past_a_proposal() {
    let mut r = setup();
    let agent = by("run/worker", 5);
    assert!(matches!(
        model::set(&mut r, &thread_key("ok"), thread::APPROVED, Some(&false), agent),
        Err(WriteError::UserOnly { .. })
    ));
    assert!(matches!(
        model::set(&mut r, &thread_key("ok"), thread::PM_FLAG, Some(&true), agent),
        Err(WriteError::UserOnly { .. })
    ));
    assert!(matches!(edit(&mut r, &member("x", None), agent).unwrap(), Outcome::Proposed(_)));
}

#[test]
fn the_user_keeps_or_drops_a_proposal_once() {
    let mut r = setup();
    let Outcome::Proposed(kept) =
        edit(&mut r, &member("y", Some("ok")), by("run/worker", 5)).unwrap()
    else {
        panic!("proposed")
    };
    let Outcome::Proposed(dropped) = edit(
        &mut r,
        &ScopeEdit::ThreadRepository {
            thread: "ok".into(),
            repository: "code".into(),
            present: true,
        },
        by("run/worker", 5),
    )
    .unwrap() else {
        panic!("proposed")
    };
    assert_eq!(
        keep(&mut r, &kept, by("run/boss", 6)),
        Err(ScopeError::NotTheUser("run/boss".into()))
    );
    keep(&mut r, &kept, by(USER, 6)).unwrap();
    drop(&mut r, &dropped, by(USER, 6)).unwrap();
    assert_eq!(keep(&mut r, &kept, by(USER, 7)), Err(ScopeError::Decided(kept.clone())));
    assert_eq!(drop(&mut r, "nope", by(USER, 7)), Err(ScopeError::NoProposal("nope".into())));
    let model = Model::read(&r.state).unwrap();
    assert_eq!(model.members("ok"), vec!["x", "y"]);
    assert!(model.threads["ok"].repositories.is_empty());
}

struct NoAncestry;

impl Ancestry for NoAncestry {
    fn merged(&self, _: &model::Repository, _: &str) -> Result<bool, String> {
        Err("none".into())
    }
}

#[derive(Debug, Clone)]
enum Step {
    Edit { replica: usize, actor: usize, item: usize, thread: Option<usize> },
    Flag { replica: usize, thread: usize, approved: bool, pm: bool },
    Decide { replica: usize, keep: bool },
    Join { into: usize, from: usize },
}

const ACTORS: [&str; 3] = [USER, "run/boss", "run/worker"];
const THREADS: [&str; 3] = ["ok", "draft", "pm"];

fn below(tc: &TestCase, n: usize) -> usize {
    tc.draw(gs::integers::<usize>().max_value(n - 1))
}

/// Apply-or-propose under concurrent flag changes: three replicas edit membership as the user, a project-management
/// run and an agent run while the user flips threads' approved and project-management flags and decides proposals.
/// Joined in any order they converge on one membership, one set of proposals and one needs-you list, and no edit is
/// lost: each either applied where it was made or exists as a proposal.
#[hegel::test(test_cases = 128)]
fn apply_or_propose_converges_and_drops_nothing(tc: TestCase) {
    let base = setup();
    let mut replicas: Vec<Replica> = ["a", "b", "c"]
        .iter()
        .map(|name| {
            let mut r = replica(name);
            r.join(&base.state).unwrap();
            r
        })
        .collect();
    let mut proposed: Vec<String> = Vec::new();
    let steps = below(&tc, 24) + 1;
    for at in 0..steps {
        let at = 10 + Millis::try_from(at).unwrap();
        let step = match below(&tc, 4) {
            0 => Step::Edit {
                replica: below(&tc, 3),
                actor: below(&tc, 3),
                item: below(&tc, 2),
                thread: (below(&tc, 4) != 0).then(|| below(&tc, 3)),
            },
            1 => Step::Flag {
                replica: below(&tc, 3),
                thread: below(&tc, 3),
                approved: below(&tc, 2) == 0,
                pm: below(&tc, 2) == 0,
            },
            2 => Step::Decide { replica: below(&tc, 3), keep: below(&tc, 2) == 0 },
            _ => Step::Join { into: below(&tc, 3), from: below(&tc, 3) },
        };
        match step {
            Step::Edit { replica, actor, item, thread } => {
                let r = &mut replicas[replica];
                let change = member(["x", "y"][item], thread.map(|t| THREADS[t]));
                match edit(r, &change, by(ACTORS[actor], at)).unwrap() {
                    Outcome::Applied => {
                        let model = Model::read(&r.state).unwrap();
                        let want = thread.map(|t| THREADS[t].to_owned());
                        assert_eq!(
                            model.items[["x", "y"][item]].thread.one().cloned(),
                            want,
                            "applied here"
                        );
                    },
                    Outcome::Proposed(id) => proposed.push(id),
                }
            },
            Step::Flag { replica, thread: t, approved, pm } => {
                let r = &mut replicas[replica];
                model::set(
                    r,
                    &thread_key(THREADS[t]),
                    thread::APPROVED,
                    Some(&approved),
                    by(USER, at),
                )
                .unwrap();
                model::set(r, &thread_key(THREADS[t]), thread::PM_FLAG, Some(&pm), by(USER, at))
                    .unwrap();
            },
            Step::Decide { replica, keep: kept } => {
                let r = &mut replicas[replica];
                let open: Vec<String> = proposals(&r.state)
                    .unwrap()
                    .into_values()
                    .filter(|p| p.decision == Value::None)
                    .map(|p| p.id)
                    .collect();
                if let Some(id) = open.first() {
                    if kept {
                        keep(r, id, by(USER, at)).unwrap();
                    } else {
                        drop(r, id, by(USER, at)).unwrap();
                    }
                }
            },
            Step::Join { into, from } => {
                let other = replicas[from].state.clone();
                replicas[into].join(&other).unwrap();
            },
        }
    }
    let order = tc.draw(gs::permutations(vec![0usize, 1, 2]));
    let mut forward = State::new();
    for &i in &order {
        forward.join(&replicas[i].state);
    }
    let mut backward = State::new();
    for &i in order.iter().rev() {
        backward.join(&replicas[i].state);
    }
    let view = |state: &State| {
        let model = Model::read(state).unwrap();
        let members: Vec<_> = THREADS.iter().map(|t| model.members(t).join(",")).collect();
        let readiness = Readiness::new(state, &model, 1_000, &NoAncestry);
        let needs = needs::needs_you(state, &model, &readiness, 1_000).unwrap();
        (members, proposals(state).unwrap(), needs)
    };
    assert_eq!(view(&forward), view(&backward));
    let all = proposals(&forward).unwrap();
    for id in &proposed {
        assert!(all.contains_key(id), "proposal {id} survives every join");
    }
}
