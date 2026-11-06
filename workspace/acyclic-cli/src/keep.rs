//! How each value of a conflict is kept, as the CLI and the app run it: the one place a [`Keep`] becomes a command,
//! both as the API's body and as the line a terminal runs.

use acyclic_core::model::Model;
use acyclic_core::needs::{Entry, Keep, Kept, Unkept};
use acyclic_core::write::{Assign, Change, ItemAssign, RepositoryAssign, ThreadAssign};
use serde_json::{Map, Value as Json, json};

use crate::commands::shell_word;
use crate::resolve::repository_name;

/// What no command keeps reads the same everywhere, and never sends the reader back to the list they came from.
const IN_A_TERMINAL: &str =
    "No command keeps this value: settle it in a terminal by writing the field you want";

/// A value of a conflict, with the command that keeps it, as `command` (the API's body) and `run` (the
/// terminal's line), or `why` none does.
pub(crate) fn kept_json(model: &Model, kept: &Kept) -> Json {
    let (command, why) = match command(model, &kept.keep) {
        Ok(command) => (Some(command), None),
        Err(why) => (None, Some(why_not(why))),
    };
    json!({
        "value": kept.value,
        "command": command.as_ref().map(Command::body),
        "run": command.as_ref().map(Command::line),
        "why": why,
    })
}

/// The exact conflict payload and executable resolution choices, without presentation text.
pub(crate) fn conflict_identity(model: &Model, entry: &Entry, kept: &[Kept]) -> String {
    let choices: Vec<Json> = kept
        .iter()
        .map(|kept| {
            json!({
                "value": &kept.value,
                "command": command(model, &kept.keep).ok().map(|command| command.body()),
            })
        })
        .collect();
    serde_json::to_string(&json!({"entry": entry, "choices": choices}))
        .expect("conflict identities serialize")
}

/// A command: its words, the record it is about, and its flags.
struct Command {
    words: [&'static str; 2],
    /// The field the API names the record by, and the record.
    subject: (&'static str, String),
    flags: Vec<(&'static str, Flag)>,
}

enum Flag {
    Text(String),
    On,
    /// A flag that may repeat, sent as a list.
    Each(String),
}

impl Command {
    fn body(&self) -> Json {
        let mut fields = Map::new();
        fields.insert(self.subject.0.to_owned(), json!(self.subject.1));
        for (flag, value) in &self.flags {
            let value = match value {
                Flag::Text(text) => json!(text),
                Flag::On => json!(true),
                Flag::Each(text) => json!([text]),
            };
            fields.insert((*flag).to_owned(), value);
        }
        let mut inner = Map::new();
        inner.insert(self.words[1].to_owned(), Json::Object(fields));
        let mut outer = Map::new();
        outer.insert(self.words[0].to_owned(), Json::Object(inner));
        Json::Object(outer)
    }

    fn line(&self) -> String {
        let mut words =
            vec!["acyclic".to_owned(), self.words[0].to_owned(), self.words[1].to_owned()];
        words.push(shell_word(&self.subject.1));
        for (flag, value) in &self.flags {
            words.push(format!("--{flag}"));
            if let Flag::Text(text) | Flag::Each(text) = value {
                words.push(shell_word(text));
            }
        }
        words.join(" ")
    }
}

/// The command a keep runs, or why there is none.
fn command(model: &Model, keep: &Keep) -> Result<Command, Unkept> {
    match keep {
        Keep::Write(changes) => match changes.0.as_slice() {
            [Change::Set(Assign::Item(id, assign))] => Ok(item_edit(id, assign)),
            [Change::Set(Assign::Thread(id, assign))] => Ok(thread_edit(id, assign)),
            [Change::Set(Assign::Repository(id, RepositoryAssign::DefaultBranch(branch)))] => {
                Ok(Command {
                    words: ["repository", "edit"],
                    subject: ("repository", id.clone()),
                    flags: vec![("default-branch", Flag::Text(branch.clone()))],
                })
            },
            [
                Change::Set(Assign::Repository(id, RepositoryAssign::Owner(owner))),
                Change::Set(Assign::Repository(_, RepositoryAssign::Name(name))),
            ] => Ok(Command {
                words: ["repository", "edit"],
                subject: ("repository", id.clone()),
                flags: vec![("name", Flag::Text(format!("{owner}/{name}")))],
            }),
            [Change::Finish { item, evidence: Some(evidence), force: _ }] => {
                let mut flags = Vec::new();
                if let Some(commit) = &evidence.commit {
                    let id = evidence.repository.as_ref().ok_or(Unkept::NoCommand)?;
                    let repository = model
                        .repositories
                        .get(id)
                        .and_then(repository_name)
                        .unwrap_or_else(|| id.clone());
                    let commit = format!("{repository}@{commit}");
                    flags.push(("commit", Flag::Text(commit)));
                }
                if let Some(text) = &evidence.text {
                    flags.push(("evidence", Flag::Text(text.clone())));
                }
                flags.push(("force", Flag::On));
                Ok(Command { words: ["item", "done"], subject: ("item", item.clone()), flags })
            },
            // `keeps` makes no other change set; a new one fails here, not silently.
            other => {
                debug_assert!(false, "no command keeps {other:?}");
                Err(Unkept::NoCommand)
            },
        },
        Keep::Not(why) => Err(why.clone()),
    }
}

fn item_edit(id: &str, assign: &ItemAssign) -> Command {
    let set = |flag: &'static str, value: &Option<String>| match value {
        Some(value) => (flag, Flag::Text(value.clone())),
        None => (clear(flag), Flag::On),
    };
    let flag = match assign {
        ItemAssign::Title(title) => ("title", Flag::Text(title.clone())),
        ItemAssign::State(state) => ("state", Flag::Text(word(state))),
        ItemAssign::Notes(notes) => set("notes", notes),
        ItemAssign::Parent(parent) => set("parent", parent),
        ItemAssign::Thread(thread) => set("thread", thread),
        ItemAssign::WaitsFor(to, true) => ("waits-for", Flag::Each(to.clone())),
        ItemAssign::WaitsFor(to, false) => ("no-waits-for", Flag::Each(to.clone())),
        // Not an edit: `keeps` keeps evidence by finishing.
        ItemAssign::Evidence(_) => {
            unreachable!("an edit doesn't write {assign:?}")
        },
    };
    Command { words: ["item", "edit"], subject: ("item", id.to_owned()), flags: vec![flag] }
}

fn thread_edit(id: &str, assign: &ThreadAssign) -> Command {
    let flag = match assign {
        ThreadAssign::Title(title) => ("title", Flag::Text(title.clone())),
        ThreadAssign::Brief(Some(brief)) => ("brief", Flag::Text(brief.clone())),
        ThreadAssign::Brief(None) => ("no-brief", Flag::On),
    };
    Command { words: ["thread", "edit"], subject: ("thread", id.to_owned()), flags: vec![flag] }
}

/// A value as the command line spells it, which is how it serializes.
fn word<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

const fn clear(flag: &str) -> &'static str {
    match flag.as_bytes() {
        b"notes" => "no-notes",
        b"parent" => "no-parent",
        _ => "no-thread",
    }
}

/// Why no command keeps a value.
fn why_not(why: Unkept) -> String {
    match why {
        Unkept::NeedsEvidence => "Keep done in a terminal: run acyclic item done with its evidence".to_owned(),
        Unkept::NoClear => "This field can't be cleared".to_owned(),
        Unkept::NotDone => "Settle its state first, then keep this evidence".to_owned(),
        Unkept::NameAlsoConflicted => {
            "Its owner and name are both written apart: settle them together with acyclic repository edit, giving \
             --name <OWNER/NAME>"
                .to_owned()
        },
        Unkept::Refused(violations) => {
            let violations: Vec<String> = violations.iter().map(ToString::to_string).collect();
            format!("Keeping it would be refused: {}", violations.join("; "))
        },
        Unkept::Unsettled | Unkept::NoCommand | Unkept::Unreadable => IN_A_TERMINAL.to_owned(),
    }
}

#[cfg(test)]
mod identity_tests {
    use acyclic_core::model::Model;
    use acyclic_core::needs::{Entry, Keep, Kept, Unkept};
    use serde_json::{Map, Value as Json, json};

    use super::conflict_identity;

    #[test]
    fn evidence_without_a_repository_offers_no_command() {
        use acyclic_core::model::{Evidence, Repository, Value};
        use acyclic_core::write::{Change, ChangeSet};

        let mut model = Model::default();
        model.repositories.insert(
            "r".into(),
            Repository {
                id: "r".into(),
                owner: Value::One("me".into()),
                name: Value::One("code".into()),
                default_branch: Value::One("main".into()),
            },
        );
        for (repository, commit, executable) in
            [(None, Some("abc1234"), false), (Some("r"), Some("abc1234"), true), (None, None, true)]
        {
            let evidence = Evidence {
                repository: repository.map(str::to_owned),
                commit: commit.map(str::to_owned),
                text: Some("Checked".into()),
            };
            let kept = Kept {
                value: json!(evidence),
                keep: Keep::Write(ChangeSet(vec![Change::Finish {
                    item: "a".into(),
                    evidence: Some(evidence),
                    force: true,
                }])),
            };
            let shown = super::kept_json(&model, &kept);
            assert_eq!(!shown["command"].is_null(), executable, "{shown}");
            assert_eq!(shown["run"].is_string(), executable, "{shown}");
            assert_eq!(shown["why"].is_string(), !executable, "{shown}");
            if repository.is_some() {
                assert_eq!(shown["command"]["item"]["done"]["commit"], "me/code@abc1234");
            }
        }
    }

    fn identity(values: Vec<Json>) -> String {
        let entry =
            Entry::Conflict { kind: "item".into(), id: "x".into(), field: "evidence".into() };
        let kept = values
            .into_iter()
            .map(|value| Kept { value, keep: Keep::Not(Unkept::Unreadable) })
            .collect::<Vec<_>>();
        conflict_identity(&Model::default(), &entry, &kept)
    }

    #[test]
    fn conflict_identity_canonicalizes_object_keys_and_preserves_array_order() {
        let mut first = Map::new();
        first.insert("z".into(), json!({"values": [1, 2]}));
        first.insert("a".into(), json!(true));
        let mut second = Map::new();
        second.insert("a".into(), json!(true));
        second.insert("z".into(), json!({"values": [1, 2]}));
        assert_eq!(identity(vec![Json::Object(first)]), identity(vec![Json::Object(second)]));

        assert_ne!(
            identity(vec![json!({"values": [1, 2]})]),
            identity(vec![json!({"values": [2, 1]})]),
            "array order is part of the conflict payload"
        );
        assert_ne!(
            identity(vec![json!("first"), json!("second")]),
            identity(vec![json!("second"), json!("first")]),
            "the ordered choice list preserves the matching between values and actions"
        );
    }
}
