use super::*;
use crate::model::{Evidence, Repository, add_item, item_key, link, set_parent};
use crate::needs::{self, Entry};
use crate::ready::Wait;
use crate::schema::{kind, repository};
use crate::state::{Key, ReplicaId, State};

const DAY: Millis = 24 * 60 * 60 * 1000;

/// Commits named `merged…` are on the default branch; any other isn't.
struct ByName;

impl Ancestry for ByName {
    fn merged(&self, _: &Repository, commit: &str) -> Result<bool, String> {
        Ok(commit.starts_with("aaaa"))
    }
}

fn user(at: Millis) -> By<'static> {
    By { actor: "actor/user", at }
}

/// A group `g` with done leaves `a` (merged) and `b` (no repository), both done on day 1.
fn setup() -> Replica {
    let mut r = Replica::open(ReplicaId::new("a").unwrap(), State::new());
    let code = Key::new(kind::REPOSITORY, "code").unwrap();
    model::set(&mut r, &code, repository::DEFAULT_BRANCH, Some(&"main"), user(1)).unwrap();
    for id in ["g", "a", "b", "outside"] {
        add_item(&mut r, id, id, user(1)).unwrap();
    }
    for id in ["a", "b"] {
        set_parent(&mut r, id, Some("g"), user(1)).unwrap();
    }
    done(&mut r, "a", Some("aaaa111"), DAY);
    done(&mut r, "b", None, DAY);
    done(&mut r, "g", None, DAY);
    r
}

fn done(r: &mut Replica, id: &str, commit: Option<&str>, at: Millis) {
    model::set_state(r, id, ItemState::Done, user(at)).unwrap();
    let evidence = Evidence {
        repository: commit.map(|_| "code".into()),
        commit: commit.map(str::to_owned),
        note: None,
    };
    model::set(r, &item_key(id).unwrap(), field::EVIDENCE, Some(&evidence), user(at)).unwrap();
}

fn reasons(r: &Replica, root: &str, now: Millis) -> Vec<Reason> {
    let model = Model::read(&r.state).unwrap();
    let readiness = Readiness::new(&r.state, &model, now, &ByName);
    eligibility(&readiness, &model, root, now).unwrap()
}

#[test]
fn a_gateless_group_waits_thirty_days_after_its_last_done() {
    let r = setup();
    assert_eq!(reasons(&r, "g", 2 * DAY), vec![Reason::Waiting { until: DAY + GATELESS_WAIT }]);
    assert_eq!(reasons(&r, "g", DAY + GATELESS_WAIT), Vec::<Reason>::new());
}

#[test]
fn a_gated_goal_is_eligible_once_done_and_merged() {
    let mut r = setup();
    model::set(&mut r, &item_key("g").unwrap(), field::GATE, Some(&"ships"), user(2)).unwrap();
    add_item(&mut r, "top", "top", user(2)).unwrap();
    set_parent(&mut r, "g", Some("top"), user(2)).unwrap();
    assert_eq!(reasons(&r, "g", 2 * DAY), Vec::<Reason>::new());
}

#[test]
fn open_unmerged_and_depended_on_subtrees_say_why() {
    let mut r = setup();
    model::set(&mut r, &item_key("g").unwrap(), field::GATE, Some(&"ships"), user(2)).unwrap();
    add_item(&mut r, "c", "c", user(2)).unwrap();
    set_parent(&mut r, "c", Some("g"), user(2)).unwrap();
    done(&mut r, "a", Some("bbbb222"), 2 * DAY);
    link(&mut r, "outside", "b", LinkKind::Blocks, user(2)).unwrap();
    assert_eq!(
        reasons(&r, "g", 3 * DAY),
        vec![
            Reason::Open { item: "c".into() },
            Reason::Unmerged { item: "a".into() },
            Reason::LiveDependent { from: "outside".into(), to: "b".into() },
        ]
    );
}

#[test]
fn archiving_refuses_until_eligible_and_then_hides_the_subtree() {
    let mut r = setup();
    let refused = archive(&mut r, &ByName, "g", user(2 * DAY)).unwrap_err();
    assert!(matches!(refused, ArchiveError::Ineligible { .. }));
    // One line, with the date the wait ends.
    assert_eq!(
        refused.to_string(),
        "g can't be archived until 1970-02-01: it has no gate, and its last item was done less than 30 days \
         ago"
    );
    let now = DAY + GATELESS_WAIT;
    let agent = By { actor: "run/worker", at: now };
    assert_eq!(
        archive(&mut r, &ByName, "g", agent),
        Err(ArchiveError::NotAllowed("run/worker".into()))
    );
    archive(&mut r, &ByName, "g", user(now)).unwrap();
    let model = Model::read(&r.state).unwrap();
    assert!(model.is_archived("a"));
    assert_eq!(reasons(&r, "g", now), vec![Reason::Archived]);
}

fn needs(r: &Replica, now: Millis) -> Vec<Entry> {
    let model = Model::read(&r.state).unwrap();
    let readiness = Readiness::new(&r.state, &model, now, &ByName);
    needs::needs_you(&r.state, &model, &readiness, now).unwrap()
}

fn status(r: &Replica, id: &str, now: Millis) -> Status {
    let model = Model::read(&r.state).unwrap();
    Readiness::new(&r.state, &model, now, &ByName).of(id).unwrap().unwrap()
}

#[test]
fn reopening_archived_work_flags_its_dependents_without_blocking_them() {
    let mut r = setup();
    link(&mut r, "outside", "a", LinkKind::Blocks, user(2)).unwrap();
    model::put(&mut r, &item_key("g").unwrap(), field::ARCHIVED, Some(&true), user(3)).unwrap();
    let now = 40 * DAY;
    reopen(&mut r, "a", user(now)).unwrap();
    let model = Model::read(&r.state).unwrap();
    assert!(!model.is_archived("a"), "the archived root above it is cleared");
    assert_eq!(status(&r, "outside", now), Status::Ready);
    assert!(
        needs(&r, now)
            .contains(&Entry::Reopened { item: "a".into(), dependents: vec!["outside".into()] })
    );
    // Done again, the flag clears.
    done(&mut r, "a", Some("aaaa111"), now + 1);
    assert!(!needs(&r, now + 1).iter().any(|e| matches!(e, Entry::Reopened { .. })));
}

#[test]
fn reopening_live_work_blocks_its_dependents_again_and_flags_them() {
    let mut r = setup();
    link(&mut r, "outside", "b", LinkKind::Blocks, user(2)).unwrap();
    assert_eq!(status(&r, "outside", 2 * DAY), Status::Ready);
    // Any actor may reopen: an agent may find a defect in finished work.
    reopen(&mut r, "b", By { actor: "run/worker", at: 2 * DAY }).unwrap();
    assert!(
        matches!(status(&r, "outside", 2 * DAY), Status::Blocked { waiting } if waiting["b"] == Wait::Open)
    );
    assert!(
        needs(&r, 2 * DAY)
            .contains(&Entry::Reopened { item: "b".into(), dependents: vec!["outside".into()] })
    );
    assert_eq!(reopen(&mut r, "b", user(3 * DAY)), Err(ArchiveError::NotDone("b".into())));
}
