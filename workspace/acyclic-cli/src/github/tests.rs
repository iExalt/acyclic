use std::cell::RefCell;
use std::rc::Rc;

use acyclic_core::model::Value;
use tempfile::TempDir;

use super::*;

fn record(name: &str) -> Repository {
    Repository {
        id: name.to_owned(),
        owner: Value::One("me".to_owned()),
        name: Value::One(name.to_owned()),
        default_branch: Value::One("main".to_owned()),
    }
}

/// A runner answering from `replies` by the repository in the endpoint, recording each call.
fn fake(replies: &[(&str, Result<String, String>)]) -> (Runner, Rc<RefCell<Vec<String>>>) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&calls);
    let replies: Vec<(String, Result<String, String>)> =
        replies.iter().map(|(name, reply)| ((*name).to_owned(), reply.clone())).collect();
    let run = move |args: &[&str]| {
        let endpoint = (*args.last().unwrap()).to_owned();
        seen.borrow_mut().push(endpoint.clone());
        replies
            .iter()
            .find(|(name, _)| endpoint.contains(&format!("/{name}/")))
            .map_or_else(|| Err("no reply".to_owned()), |(_, reply)| reply.clone())
    };
    (Box::new(run), calls)
}

fn status(status: &str) -> String {
    format!("{{\"status\":\"{status}\"}}")
}

#[test]
fn only_a_merged_verdict_is_cached_and_each_commit_is_asked_once() {
    let temp = TempDir::new().unwrap();
    let cache = temp.path().join("merged.json");
    let (run, calls) = fake(&[
        ("done", Ok(status("behind"))),
        ("open", Ok(status("ahead"))),
        ("lost", Err("gh: Not Found (HTTP 404)".to_owned())),
    ]);
    let github = Github::with(run, Some(cache.clone()));
    for _ in 0..3 {
        assert_eq!(github.check(&record("done"), "abcdef1"), Check::Merged);
        assert_eq!(github.check(&record("open"), "abcdef1"), Check::NotMerged);
        assert!(matches!(github.check(&record("lost"), "abcdef1"), Check::Unchecked(_)));
    }
    assert_eq!(calls.borrow().len(), 3, "{:?}", calls.borrow());
    let cached = std::fs::read_to_string(&cache).unwrap();
    assert!(cached.contains("me/done@main@abcdef1"), "{cached}");
    assert!(!cached.contains("open") && !cached.contains("lost"), "{cached}");

    // Another command knows the merge without asking, and still asks about the rest.
    let (run, calls) = fake(&[("open", Ok(status("ahead")))]);
    let github = Github::with(run, Some(cache));
    assert_eq!(github.check(&record("done"), "ABCDEF1"), Check::Merged);
    assert!(calls.borrow().is_empty());
    assert_eq!(github.check(&record("open"), "abcdef1"), Check::NotMerged);
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn after_the_first_transport_failure_nothing_more_is_asked() {
    let (run, calls) =
        fake(&[("a", Err("gh: authentication required".to_owned())), ("b", Ok(status("behind")))]);
    let github = Github::with(run, None);
    let first = github.check(&record("a"), "abcdef1");
    let Check::Unchecked(reason) = first else { panic!("{first:?}") };
    assert!(reason.contains("authentication required"), "{reason}");
    let second = github.check(&record("b"), "abcdef1");
    let Check::Unchecked(reason) = second else { panic!("{second:?}") };
    assert!(reason.contains("authentication required"), "{reason}");
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn a_repository_github_cannot_show_does_not_stop_the_others() {
    let (run, calls) =
        fake(&[("a", Err("gh: Not Found (HTTP 404)".to_owned())), ("b", Ok(status("behind")))]);
    let github = Github::with(run, None);
    assert!(matches!(github.check(&record("a"), "abcdef1"), Check::Unchecked(_)));
    assert_eq!(github.check(&record("b"), "abcdef1"), Check::Merged);
    assert_eq!(calls.borrow().len(), 2);
}

#[test]
fn without_a_path_there_is_no_gh_to_run() {
    let env = Env { vars: BTreeMap::new(), cwd: PathBuf::from("/"), terminal: None };
    let said = runner(&env)(&["api"]).unwrap_err();
    assert!(said.contains("gh isn't installed"), "{said}");
}
