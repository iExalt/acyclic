use super::*;

#[test]
fn a_new_id_is_a_lowercase_v7_and_ids_made_later_sort_later() {
    let id = new();
    assert_eq!(v7(&id).as_deref(), Some(id.as_str()));
    let (early, late) = (at(1_000, [0xff; 10]), at(1_001, [0; 10]));
    assert!(early < late, "{early} {late}");
    assert_eq!(v7(&early).as_deref(), Some(early.as_str()));
}

#[test]
fn only_a_full_v7_in_any_case_is_an_id() {
    let id = at(1_700_000_000_000, [7; 10]);
    assert_eq!(v7(&id.to_uppercase()), Some(id.clone()));
    for not in [
        "plant",
        "",
        &id[1..],
        &id.replace('-', ""),
        &format!("{{{id}}}"),
        // A version 4 UUID.
        "6f1c1e2a-3b4d-4c5e-8f60-718293a4b5c6",
        // Version 7 with another variant.
        &format!("{}c{}", &id[..19], &id[20..]),
    ] {
        assert_eq!(v7(not), None, "{not}");
    }
    assert!(is_full("6F1C1E2A-3B4D-4C5E-8F60-718293A4B5C6"));
    assert!(!is_full("6f1c1e2a-3b4d-4c5e-8f60-718293a4b5c"));
}
