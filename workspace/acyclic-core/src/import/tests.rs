use super::*;

/// A fixed `UUIDv7` told apart by `n`.
pub(super) fn id(n: u8) -> String {
    format!("01900000-0000-7000-8000-{n:012x}")
}

pub(super) fn item(n: u8, state: Option<State>, parent: Option<u8>) -> Item {
    Item {
        id: id(n),
        title: format!("Item {n}"),
        notes: None,
        state,
        parent: parent.map(id),
        thread: None,
        waits_for: None,
        evidence: None,
    }
}

/// Repository 1; items 10, 11 under 10 and 12 done under 10 in repository 1, 11 waiting for 12; thread 20 holding
/// 11 and 12.
pub(super) fn valid() -> Import {
    let mut import = Import::new();
    import.repositories.push(Repository {
        id: id(1),
        name: "example/code".to_owned(),
        default_branch: "main".to_owned(),
    });
    import.items = vec![
        item(10, None, None),
        item(11, Some(State::Backlog), Some(10)),
        item(12, Some(State::Done), Some(10)),
    ];
    import.items[1].waits_for = Some(vec![id(12)]);
    import.items[2].evidence =
        Some(Evidence { commit: Some(format!("{}@abc1234", id(1))), text: Some("shipped".into()) });
    for member in &mut import.items[1..] {
        member.thread = Some(id(20));
    }
    import.threads = vec![Thread { id: id(20), title: "Thread".into(), notes: None }];
    import
}

fn document(version: u32, items: &str) -> String {
    format!("{{\"format\":\"{FORMAT}\",\"version\":{version},\"items\":[{items}]}}")
}

#[test]
fn a_valid_document_round_trips() {
    let import = valid();
    assert_eq!(import.validate(), vec![]);
    assert_eq!(Import::from_json(&import.to_json()).unwrap(), import);
}

#[test]
fn only_version_3_is_read() {
    assert!(matches!(Import::from_json("[]"), Err(ParseError::NotAnImport)));
    assert!(matches!(
        Import::from_json("{\"format\":\"other\",\"version\":3}"),
        Err(ParseError::NotAnImport)
    ));
    for version in [0, 1, 2, 4] {
        assert!(
            matches!(
                Import::from_json(&document(version, "")),
                Err(ParseError::UnsupportedVersion { found }) if found == u64::from(version)
            ),
            "{version}"
        );
    }
    assert_eq!(Import::from_json(&document(3, "")).unwrap(), Import::new());
}

#[test]
fn a_state_reads_open_or_the_stored_backlog() {
    for word in ["open", "backlog"] {
        let doc =
            document(3, &format!("{{\"id\":\"{}\",\"title\":\"A\",\"state\":\"{word}\"}}", id(1)));
        assert_eq!(Import::from_json(&doc).unwrap().items[0].state, Some(State::Backlog), "{word}");
    }
}

#[test]
fn removed_and_unknown_fields_are_refused() {
    let one =
        |fields: &str| document(3, &format!("{{\"id\":\"{}\",\"title\":\"A\"{fields}}}", id(1)));
    for removed in [
        r#","key":"a""#,
        r#","areas":["x"]"#,
        r#","state":"in-progress""#,
        r#","state":"done","evidence":{"note":"shipped"}"#,
        r#","state":"done","evidence":{"repository":"r","commit":"abc1234"}"#,
    ] {
        assert!(matches!(Import::from_json(&one(removed)), Err(ParseError::Json(_))), "{removed}");
    }
    // A retired key is refused by name, not dropped.
    for retired in ["size", "archived"] {
        let err = Import::from_json(&one(&format!(r#","{retired}":"m""#))).unwrap_err();
        assert!(err.to_string().contains(&format!("unknown field `{retired}`")), "{err}");
    }
    for (list, record, retired) in [
        ("items", r#""title":"A","repositories":[]"#, "repositories"),
        ("threads", r#""title":"T","repositories":[]"#, "repositories"),
        (
            "repositories",
            r#""name":"a/b","default_branch":"main","installation":"1""#,
            "installation",
        ),
    ] {
        let doc = format!(
            r#"{{"format":"{FORMAT}","version":3,"{list}":[{{"id":"{}",{record}}}]}}"#,
            id(1)
        );
        let err = Import::from_json(&doc).unwrap_err();
        assert!(err.to_string().contains(&format!("unknown field `{retired}`")), "{list}: {err}");
    }
    let threads = format!(
        "{{\"format\":\"{FORMAT}\",\"version\":3,\"threads\":[{{\"id\":\"{}\",\"title\":\"T\",\"items\":[]}}]}}",
        id(2)
    );
    assert!(matches!(Import::from_json(&threads), Err(ParseError::Json(_))), "a thread's items");
}

#[test]
fn every_id_and_reference_is_a_uuidv7_in_any_case() {
    let mut import = valid();
    import.items[0].id = "plant".into();
    import.items[1].waits_for = Some(vec!["6f1c1e2a-3b4d-4c5e-8f60-718293a4b5c6".into()]);
    let errors = import.validate();
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors.iter().all(ValidationError::is_invalid_id));
    // Upper case names the same record, lowercased as it is read.
    let upper = valid().to_json().replace(&id(12), &id(12).to_uppercase());
    assert_eq!(Import::from_json(&upper).unwrap(), valid());
}

#[test]
fn the_schema_example_is_a_valid_document() {
    let json: String = SCHEMA.lines().filter(|line| !line.trim_start().starts_with("//")).collect();
    let example = Import::from_json(&json).unwrap();
    assert_eq!(example.validate(), vec![]);
    assert_eq!(example.version, VERSION);
    assert!(SCHEMA.contains("acyclic id") && SCHEMA.contains("single quotes"));
}

#[test]
fn ids_are_unique_within_a_kind() {
    let mut import = valid();
    import.items.push(item(10, None, None));
    // A thread may share an item's id: they are different kinds.
    import.threads[0].id = id(10);
    for member in &mut import.items {
        member.thread = None;
    }
    assert_eq!(import.validate(), vec![ValidationError::DuplicateId(KeyKind::Item, id(10))]);
}

#[test]
fn a_waits_for_list_names_another_item_once() {
    let mut import = valid();
    import.items[1].waits_for = Some(vec![id(12), id(12), id(11)]);
    assert_eq!(
        import.validate(),
        vec![
            ValidationError::SelfLink(id(11)),
            ValidationError::DuplicateLink { item: id(11), waits_for: id(12) },
        ]
    );
}

#[test]
fn a_repository_is_owner_and_name_and_a_commit_names_its_repository() {
    let mut import = valid();
    import.repositories[0].name = "code".into();
    import.items[2].evidence.as_mut().unwrap().commit = Some("abc1234".into());
    assert_eq!(
        import.validate(),
        vec![
            ValidationError::InvalidRepositoryName { repository: id(1), name: "code".into() },
            ValidationError::InvalidCommit { item: id(12), commit: "abc1234".into() },
        ]
    );
    assert_eq!(owner_name("me/code"), Some(("me", "code")));
    for not in ["me/", "/code", "me/a/b", "me /code", "code"] {
        assert_eq!(owner_name(not), None, "{not}");
    }
}
