//! The usage ledger: one record per model call, kept in each writer's usage segments
//! (`refs/heads/usage/<replica>/<yyyy-mm>/<generation>`) beside its audit log, and never in state. A record carries
//! tokens by class and any cost the provider reported; prices are applied when reading, so a corrected price table
//! recomputes every record it covers.
//!
//! Costs are exact: rates are whole millionths of a dollar per million tokens, so a cost is a whole number of
//! millionths of a millionth of a dollar ([`Picos`]). How a provider rounds its own bill, per call or per total, is
//! left to whoever compares the two.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::Millis;

/// Tokens by class.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tokens {
    /// Input tokens, not counting cache reads or writes.
    pub input: u64,
    /// Output tokens, reasoning included.
    pub output: u64,
    /// Tokens read from the cache.
    pub cache_read: u64,
    /// Tokens written to the cache for five minutes.
    pub cache_write_5m: u64,
    /// Tokens written to the cache for an hour.
    pub cache_write_1h: u64,
    /// The part of [`Self::output`] spent reasoning, where the provider reports it: for information, never priced
    /// again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
    /// Classes this version doesn't know, kept as they were and left unpriced.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One model call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The writer's stable id for the call, such as `<run>/<sequence>`: a copy arriving twice has the same id.
    pub id: String,
    /// When the call was made.
    pub at: Millis,
    /// The run that made it, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// The thread it worked for, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The project root it worked in, if any; it names the record's file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// The item it worked on, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// The provider, such as `anthropic`.
    pub provider: String,
    /// The model as the provider names it.
    pub model: String,
    /// A label for the credential that paid, such as `inference-key`: never key material.
    pub credential: String,
    /// The call's class, such as the main loop or a classifier, where known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// Tokens by class.
    #[serde(default)]
    pub tokens: Tokens,
    /// The cost the provider reported, in millionths of a dollar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_cost: Option<u64>,
    /// Any per-call price dimension the provider reports, such as a service tier or region.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dimensions: BTreeMap<String, String>,
    /// Whether the call ended without reporting its usage, so its tokens are a lower bound.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub usage_missing: bool,
    /// Fields this version doesn't know, kept as they were.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Record {
    /// The project root whose file holds the record: its project, or `_` for none.
    #[must_use]
    pub fn root(&self) -> &str {
        self.project.as_deref().unwrap_or("_")
    }
}

/// A record as read, with the replica that wrote it.
#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    /// The writer.
    pub writer: String,
    /// The record.
    pub record: Record,
}

/// The ledger as read from every writer's segments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ledger {
    /// Each call once, by id: of several copies, the earliest, then the one from the lowest writer id.
    pub records: Vec<Written>,
    /// Lines that couldn't be read, as `<ref>:<path>:<line>`.
    pub unreadable: Vec<String>,
}

/// Keeps one copy of each record by id: the earliest, then the one from the lowest writer id, so every replica keeps
/// the same copy whatever order it read them in. The result is ordered by time, then id.
#[must_use]
pub fn dedup(copies: impl IntoIterator<Item = Written>) -> Vec<Written> {
    let mut kept: BTreeMap<String, Written> = BTreeMap::new();
    for copy in copies {
        let key = |w: &Written| (w.record.at, w.writer.clone());
        match kept.get(&copy.record.id) {
            Some(existing) if key(existing) <= key(&copy) => {},
            _ => {
                kept.insert(copy.record.id.clone(), copy);
            },
        }
    }
    let mut records: Vec<Written> = kept.into_values().collect();
    records.sort_by(|a, b| (a.record.at, &a.record.id).cmp(&(b.record.at, &b.record.id)));
    records
}

/// Millionths of a millionth of a dollar: an exact cost.
pub type Picos = u128;

/// Prices per million tokens of each class, in millionths of a dollar: $3 per million is `3_000_000`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rates {
    /// Input tokens.
    pub input: u64,
    /// Output tokens, reasoning included.
    pub output: u64,
    /// Cache reads.
    pub cache_read: u64,
    /// Cache writes kept five minutes.
    pub cache_write_5m: u64,
    /// Cache writes kept an hour.
    pub cache_write_1h: u64,
}

/// A model's rates from a time on, until a later rate for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rate {
    /// The provider.
    pub provider: String,
    /// The model.
    pub model: String,
    /// When the rates start to apply.
    pub from: Millis,
    /// The rates.
    pub per_mtok: Rates,
}

/// A price table. A new rate for a model from some time on is a change and leaves calls before it alone; a new
/// version of the table that alters a rate already in force is a correction and recomputes every call it covers,
/// since costs are never stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prices {
    /// The table's version, named beside every total priced by it.
    pub version: u32,
    /// The rates.
    pub rates: Vec<Rate>,
}

impl Prices {
    /// The rate for a call: its provider's and model's latest to start at or before the call, and of two starting
    /// together, the later in the table.
    #[must_use]
    pub fn rate(&self, record: &Record) -> Option<&Rate> {
        self.rates
            .iter()
            .filter(|rate| {
                rate.provider == record.provider
                    && rate.model == record.model
                    && rate.from <= record.at
            })
            .max_by_key(|rate| rate.from)
    }

    /// A call's cost at its rate, or `None` when the table can't price all of it: no rate for it, a price
    /// dimension such as a service tier (rates don't distinguish them yet), or tokens of a class this version doesn't
    /// know. Reasoning is part of output, so it isn't priced again.
    #[must_use]
    pub fn cost(&self, record: &Record) -> Option<Picos> {
        let unknown = record.tokens.extra.values().any(|count| count.as_u64() != Some(0));
        if unknown || !record.dimensions.is_empty() {
            return None;
        }
        let rates = self.rate(record)?.per_mtok;
        let tokens = &record.tokens;
        Some(
            [
                (tokens.input, rates.input),
                (tokens.output, rates.output),
                (tokens.cache_read, rates.cache_read),
                (tokens.cache_write_5m, rates.cache_write_5m),
                (tokens.cache_write_1h, rates.cache_write_1h),
            ]
            .into_iter()
            .map(|(count, rate)| Picos::from(count) * Picos::from(rate))
            .sum(),
        )
    }
}

/// Costs summed by some key, such as the month, the run or the credential.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Totals {
    /// The price table's version.
    pub version: u32,
    /// The cost of the priced calls, by key.
    pub cost: BTreeMap<String, Picos>,
    /// The calls the table couldn't price, by key: never counted as free.
    pub unpriced: BTreeMap<String, usize>,
    /// The calls that ended without reporting their usage, by key, priced or not: their cost is a lower bound.
    pub incomplete: BTreeMap<String, usize>,
}

/// Prices every record and sums the costs by `key`.
pub fn totals<'a>(
    records: impl IntoIterator<Item = &'a Record>,
    prices: &Prices,
    key: impl Fn(&Record) -> String,
) -> Totals {
    let mut totals = Totals { version: prices.version, ..Totals::default() };
    for record in records {
        if record.usage_missing {
            *totals.incomplete.entry(key(record)).or_default() += 1;
        }
        match prices.cost(record) {
            Some(cost) => *totals.cost.entry(key(record)).or_default() += cost,
            None => *totals.unpriced.entry(key(record)).or_default() += 1,
        }
    }
    totals
}

#[cfg(test)]
mod tests;
