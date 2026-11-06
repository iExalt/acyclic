use hegel::TestCase;
use hegel::generators as gs;

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

#[test]
fn a_loose_tasks_thread_is_its_id_and_one_and_nothing_else_is_derived() {
    let task = at(1_700_000_000_000, [7; 10]);
    assert_eq!(one(&task), "018bcfe5-6800-7707-8707-070707070707.one");
    assert_eq!(task_of_one(&one(&task)), Some(task.as_str()));
    for not in [&task, "", ".one", "one", "x.once", "x.one.two"] {
        assert_eq!(task_of_one(not), None, "{not}");
    }
}

#[hegel::test]
fn deriving_a_thread_of_one_is_pure_and_never_collides_with_a_minted_id(tc: TestCase) {
    let millis = tc.draw(gs::integers::<u64>().max_value(0xffff_ffff_ffff));
    let bits = tc.draw(gs::integers::<u64>());
    let mut random = [0; 10];
    random[..8].copy_from_slice(&bits.to_le_bytes());
    let task = at(millis, random);
    let derived = one(&task);
    assert_eq!(derived, one(&task));
    assert!(crate::state::Key::new(crate::schema::kind::THREAD, &derived).is_ok());
    assert!(!is_full(&derived) && !derived.contains(':'), "{derived}");
    assert_eq!(task_of_one(&derived), Some(task.as_str()));
    // Two tasks never share a thread of one, and it is never a task's own id.
    let other = at(millis.wrapping_add(1) & 0xffff_ffff_ffff, [1; 10]);
    assert_eq!(derived == one(&other), task == other);
    assert_ne!(derived, task);
}
