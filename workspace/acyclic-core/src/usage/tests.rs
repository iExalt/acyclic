use hegel::TestCase;
use hegel::generators as gs;
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

fn priced(id: &str, at: Millis, model: &str, tokens: &serde_json::Value) -> Record {
    serde_json::from_value(json!({
        "id": id, "at": at, "provider": "anthropic", "model": model, "credential": "test-key", "tokens": tokens,
    }))
    .unwrap()
}

fn rate(model: &str, from: Millis, input: u64, output: u64) -> Rate {
    Rate {
        provider: "anthropic".into(),
        model: model.into(),
        from,
        per_mtok: Rates { input, output, cache_read: 30, cache_write_5m: 50, cache_write_1h: 70 },
    }
}

/// $1 in picodollars.
const DOLLAR: Picos = 1_000_000_000_000;

#[test]
fn costs_are_exact_and_reasoning_is_part_of_output() {
    let prices = Prices { version: 1, rates: vec![rate("m", 0, 3_000_000, 75_000)] };
    let call = priced(
        "a",
        10,
        "m",
        &json!({"input": 1_000_000, "output": 1, "reasoning": 1, "cache_read": 2, "cache_write_5m": 3,
                "cache_write_1h": 4, "audio": 0}),
    );
    // $3 for a million input tokens, $0.000000075 for one output token, and the cache classes at their own rates;
    // the reasoning token is already in output.
    assert_eq!(prices.cost(&call), Some(3 * DOLLAR + 75_000 + 2 * 30 + 3 * 50 + 4 * 70));
}

#[test]
fn what_the_table_cant_price_in_full_is_unpriced_never_free() {
    let prices = Prices { version: 1, rates: vec![rate("m", 0, 3_000_000, 75_000)] };
    let call = priced("a", 10, "m", &json!({"input": 1}));
    assert!(prices.cost(&call).is_some());
    // A model the table doesn't name, or a call before its first rate.
    assert_eq!(prices.cost(&priced("b", 10, "other", &json!({"input": 1}))), None);
    let later = Prices { version: 1, rates: vec![rate("m", 11, 3_000_000, 75_000)] };
    assert_eq!(later.cost(&call), None);
    // Tokens of a class this version doesn't know, or a price dimension the rates don't distinguish.
    assert_eq!(prices.cost(&priced("c", 10, "m", &json!({"input": 1, "audio": 9}))), None);
    let mut tiered = call.clone();
    tiered.dimensions.insert("service_tier".into(), "priority".into());
    assert_eq!(prices.cost(&tiered), None);
    // Totals keep them apart, and count calls whose usage never arrived, priced or not.
    let mut missing = call.clone();
    missing.usage_missing = true;
    let totals = totals([&call, &tiered, &missing], &prices, |r| r.model.clone());
    assert_eq!(totals.cost.get("m"), Some(&(2 * 3_000_000)));
    assert_eq!(totals.unpriced, BTreeMap::from([("m".to_owned(), 1)]));
    assert_eq!(totals.incomplete, BTreeMap::from([("m".to_owned(), 1)]));
}

#[test]
fn a_correction_recomputes_past_calls_and_a_change_only_later_ones() {
    let calls = [
        priced("a", 100, "m", &json!({"input": 10})),
        priced("b", 200, "m", &json!({"input": 10})),
    ];
    let total = |prices: &Prices| totals(&calls, prices, |_| "all".to_owned());
    let first = Prices { version: 1, rates: vec![rate("m", 0, 2, 0)] };
    assert_eq!(total(&first).cost["all"], 40);
    // A change from 150 on: the call before it keeps its price.
    let changed = Prices { version: 2, rates: vec![rate("m", 0, 2, 0), rate("m", 150, 5, 0)] };
    assert_eq!(total(&changed).cost["all"], 20 + 50);
    // A correction of the rate in force all along: both calls are priced anew, under the new version.
    let corrected = Prices { version: 3, rates: vec![rate("m", 0, 4, 0), rate("m", 150, 5, 0)] };
    assert_eq!((total(&corrected).version, total(&corrected).cost["all"]), (3, 40 + 50));
}

/// However many copies arrive, from whichever writers in whatever order, the totals are those of one copy of each
/// call.
#[hegel::test]
fn totals_ignore_copies_and_order(tc: TestCase) {
    let prices = Prices { version: 1, rates: vec![rate("m", 0, 3, 7), rate("m", 50, 4, 9)] };
    let calls: Vec<Record> = (0..tc.draw(gs::integers::<usize>().max_value(8)))
        .map(|n| {
            let at = tc.draw(gs::integers::<Millis>().min_value(0).max_value(100));
            let tokens = json!({"input": tc.draw(gs::integers::<u64>().max_value(1_000_000)),
                                "output": tc.draw(gs::integers::<u64>().max_value(1_000_000))});
            priced(&format!("c{n}"), at, "m", &tokens)
        })
        .collect();
    let mut copies = Vec::new();
    for _ in 0..tc.draw(gs::integers::<usize>().max_value(20)) {
        if calls.is_empty() {
            break;
        }
        let call = &calls[tc.draw(gs::integers::<usize>().max_value(calls.len() - 1))];
        let writer = ["mac", "server", "phone"][tc.draw(gs::integers::<usize>().max_value(2))];
        copies.push(Written { writer: writer.to_owned(), record: call.clone() });
    }
    let arrived: BTreeMap<&str, &Record> =
        copies.iter().map(|w| (w.record.id.as_str(), &w.record)).collect();
    let once = totals(arrived.values().copied(), &prices, |_| "all".to_owned());
    let kept = dedup(copies);
    assert_eq!(totals(kept.iter().map(|w| &w.record), &prices, |_| "all".to_owned()), once);
}
