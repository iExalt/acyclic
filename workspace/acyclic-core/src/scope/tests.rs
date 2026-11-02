use super::*;
use crate::model::{Model, add_item};
use crate::state::{ReplicaId, State};

/// An agent moves an item between threads, and takes it out of one, with no refusal and nothing held for the user.
#[test]
fn an_agent_moves_an_item_between_threads() {
    let mut r = Replica::open(ReplicaId::new("base").unwrap(), State::new());
    add_item(&mut r, "x", "x", By { actor: USER, at: 1 }).unwrap();
    let agent = By { actor: "session/x", at: 2 };
    let member = |thread: Option<&str>| ScopeEdit::Member {
        item: "x".to_owned(),
        thread: thread.map(str::to_owned),
    };
    for thread in [Some("a"), Some("b"), None] {
        edit(&mut r, &member(thread), agent).unwrap();
        let model = Model::read(&r.state).unwrap();
        assert_eq!(model.items["x"].thread.one().map(String::as_str), thread);
    }
}
