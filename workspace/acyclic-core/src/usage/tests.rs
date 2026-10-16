use serde_json::json;

use super::*;

fn copy(writer: &str, id: &str, at: Millis) -> Written {
    let record = serde_json::from_value(json!({
        "id": id, "at": at, "provider": "anthropic", "model": "claude-test", "credential": "test-key",
    }))
    .unwrap();
    Written { writer: writer.to_owned(), record }
}

fn kept(copies: Vec<Written>) -> Vec<(String, String, Millis)> {
    dedup(copies).into_iter().map(|w| (w.writer, w.record.id, w.record.at)).collect()
}

#[test]
fn each_call_counts_once_keeping_the_same_copy_on_every_replica() {
    let copies = vec![
        copy("server", "r1/1", 5),
        copy("mac", "r1/1", 5),
        copy("mac", "r1/1", 5),
        copy("mac", "r1/2", 9),
        copy("server", "r1/2", 7),
        copy("phone", "r1/3", 1),
    ];
    let expected = vec![
        ("phone".to_owned(), "r1/3".to_owned(), 1),
        ("mac".to_owned(), "r1/1".to_owned(), 5),
        ("server".to_owned(), "r1/2".to_owned(), 7),
    ];
    assert_eq!(kept(copies.clone()), expected);
    let mut reversed = copies;
    reversed.reverse();
    assert_eq!(kept(reversed), expected);
}

#[test]
fn a_record_round_trips_with_fields_it_doesnt_know() {
    let text = json!({
        "id": "r1/1", "at": 1, "provider": "anthropic", "model": "claude-test", "credential": "test-key",
        "tokens": {"input": 3, "output": 2, "reasoning": 1, "audio": 4},
        "dimensions": {"service_tier": "standard"}, "future": [1, 2],
    });
    let record: Record = serde_json::from_value(text.clone()).unwrap();
    assert_eq!(record.root(), "_");
    assert_eq!(record.tokens.reasoning, Some(1));
    // Unknown fields are kept, token classes included, though a reader can't price a class it doesn't know.
    let mut expected = text;
    expected["tokens"] = json!({"input": 3, "output": 2, "cache_read": 0, "cache_write_5m": 0, "cache_write_1h": 0,
                                "reasoning": 1, "audio": 4});
    assert_eq!(serde_json::to_value(&record).unwrap(), expected);
}
