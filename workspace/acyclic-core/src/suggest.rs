//! Placement candidates: where a new item might go. Live items are ranked as parents and threads by how much their
//! words share with the item's (TF-IDF cosine over titles and notes, with a group's children's titles describing it),
//! by shared areas and repositories, and by closeness to what the item links to. Archived items are never parents;
//! one that matches well is offered as a `relates` link beside a live parent instead.
//!
//! The weights are first guesses, named so that later tuning has a reference.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::model::{Item, Model};
use crate::place;

/// Candidates of each kind shown on an add.
pub const LIMIT: usize = 3;
/// How much a child's title counts toward its parent's words, against the parent's own.
pub const CHILD_WEIGHT: f64 = 0.5;
/// Added for each area or repository a candidate shares with the item.
pub const LABEL_BONUS: f64 = 0.2;
/// Added to the parent of each item the item links to; half of it to the grandparent.
pub const PROXIMITY: f64 = 0.3;

/// Words that say nothing about where work belongs.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "into", "is", "it",
    "its", "no", "not", "of", "on", "or", "that", "the", "this", "to", "was", "with",
];

/// What an item brings to be placed: its words, labels and links.
#[derive(Debug, Clone, Default)]
pub struct Query<'a> {
    /// Its title.
    pub title: &'a str,
    /// Its notes.
    pub notes: &'a str,
    /// Its areas.
    pub areas: BTreeSet<&'a str>,
    /// Its repositories.
    pub repositories: BTreeSet<&'a str>,
    /// The items it links to, either way.
    pub links: BTreeSet<&'a str>,
}

impl<'a> Query<'a> {
    /// An item's own words, labels and links, as an add with links or an import sees them.
    #[must_use]
    pub fn of(model: &'a Model, item: &'a Item) -> Self {
        let mut query = Self::words_of(item);
        for link in &model.links {
            if link.from == item.id {
                query.links.insert(&link.to);
            } else if link.to == item.id {
                query.links.insert(&link.from);
            }
        }
        query
    }

    /// An item's words and labels alone, as a fresh add sees them.
    #[must_use]
    pub fn words_of(item: &'a Item) -> Self {
        Self {
            title: item.title.one().map_or("", String::as_str),
            notes: item.notes.one().map_or("", String::as_str),
            areas: item.areas.iter().map(String::as_str).collect(),
            repositories: item.repositories.iter().map(String::as_str).collect(),
            links: BTreeSet::new(),
        }
    }
}

/// Every candidate of each kind, best first, scored and rounded to three decimals.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ranking {
    /// Live items, as parents.
    pub parents: Vec<(String, f64)>,
    /// Threads with a score above zero.
    pub threads: Vec<(String, f64)>,
    /// Archived items with a score above zero.
    pub archived: Vec<(String, f64)>,
}

/// The lowercased words of `text`: letters and digits, with dots inside a word kept so ids such as `2.4` stay whole;
/// single characters and stopwords dropped.
#[must_use]
pub fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '.')
        .map(|word| word.trim_matches('.').to_lowercase())
        .filter(|word| word.chars().count() > 1 && !STOPWORDS.contains(&word.as_str()))
        .collect()
}

type Vector = BTreeMap<String, f64>;

fn add_words(vector: &mut Vector, text: &str, weight: f64) {
    for token in tokens(text) {
        *vector.entry(token).or_default() += weight;
    }
}

fn title_of(item: &Item) -> &str {
    item.title.one().map_or("", String::as_str)
}

fn notes_of(item: &Item) -> &str {
    item.notes.one().map_or("", String::as_str)
}

/// Rounded to three decimals, so the order can't depend on float noise.
fn round(score: f64) -> f64 {
    (score * 1000.0).round() / 1000.0
}

fn sorted(mut scores: Vec<(String, f64)>) -> Vec<(String, f64)> {
    for entry in &mut scores {
        entry.1 = round(entry.1);
    }
    scores.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scores
}

/// Scores word vectors and labels against one query.
struct Scorer<'q> {
    query: &'q Query<'q>,
    /// In how many documents each word appears.
    frequency: BTreeMap<String, usize>,
    documents: usize,
    /// The query's words, weighed.
    wanted: Vector,
}

impl<'q> Scorer<'q> {
    fn new(query: &'q Query<'q>, items: &[&Item]) -> Self {
        let mut frequency: BTreeMap<String, usize> = BTreeMap::new();
        for item in items {
            let words: BTreeSet<String> =
                tokens(title_of(item)).into_iter().chain(tokens(notes_of(item))).collect();
            for word in words {
                *frequency.entry(word).or_default() += 1;
            }
        }
        let mut scorer = Self { query, frequency, documents: items.len(), wanted: Vector::new() };
        let mut wanted = Vector::new();
        add_words(&mut wanted, query.title, 1.0);
        add_words(&mut wanted, query.notes, 1.0);
        scorer.wanted = scorer.weigh(wanted);
        scorer
    }

    /// Term frequencies times inverse document frequency, scaled to unit length.
    #[expect(clippy::cast_precision_loss, reason = "item counts are far below 2^52")]
    fn weigh(&self, vector: Vector) -> Vector {
        let n = self.documents as f64;
        let weighted: Vector = vector
            .into_iter()
            .map(|(word, tf)| {
                let df = self.frequency.get(&word).copied().unwrap_or(0) as f64;
                let idf = ((n + 1.0) / (df + 1.0)).ln() + 1.0;
                (word, tf * idf)
            })
            .collect();
        let norm = weighted.values().map(|w| w * w).sum::<f64>().sqrt();
        if norm == 0.0 {
            weighted
        } else {
            weighted.into_iter().map(|(word, w)| (word, w / norm)).collect()
        }
    }

    /// The cosine between the query's words and these.
    fn words(&self, vector: Vector) -> f64 {
        let vector = self.weigh(vector);
        self.wanted.iter().map(|(word, w)| w * vector.get(word).copied().unwrap_or(0.0)).sum()
    }

    /// The bonus for the areas and repositories these share with the query.
    fn labels<'s>(
        &self,
        areas: impl Iterator<Item = &'s String>,
        repositories: impl Iterator<Item = &'s String>,
    ) -> f64 {
        let areas: BTreeSet<&str> = areas.map(String::as_str).collect();
        let repositories: BTreeSet<&str> = repositories.map(String::as_str).collect();
        let shared = self.query.areas.intersection(&areas).count()
            + self.query.repositories.intersection(&repositories).count();
        #[expect(clippy::cast_precision_loss, reason = "a handful of labels")]
        let shared = shared as f64;
        shared * LABEL_BONUS
    }
}

/// Ranks candidates for `query`, leaving out `exclude` (the item and its subtree): they are never candidates, and
/// their words never describe a candidate or weigh a term.
#[must_use]
pub fn rank(model: &Model, query: &Query<'_>, exclude: &BTreeSet<&str>) -> Ranking {
    let items: Vec<&Item> =
        model.items.values().filter(|item| !exclude.contains(item.id.as_str())).collect();
    let scorer = Scorer::new(query, &items);
    let mut near: BTreeMap<&str, f64> = BTreeMap::new();
    let parents_of = |id: &str| -> Vec<&str> {
        let parent = model.items.get(id).map(|item| item.parent.all()).unwrap_or_default();
        parent.into_iter().map(String::as_str).collect()
    };
    // Every value of a conflicted parent counts as a parent.
    for linked in query.links.iter().filter(|id| !exclude.contains(**id)) {
        for parent in parents_of(linked) {
            *near.entry(parent).or_default() += PROXIMITY;
            for grandparent in parents_of(parent) {
                *near.entry(grandparent).or_default() += PROXIMITY / 2.0;
            }
        }
    }
    let (mut parents, mut archived) = (Vec::new(), Vec::new());
    for item in &items {
        let children: Vec<&Item> = model
            .children(&item.id)
            .into_iter()
            .filter(|child| !exclude.contains(child))
            .filter_map(|child| model.items.get(child))
            .collect();
        let mut words = Vector::new();
        add_words(&mut words, title_of(item), 1.0);
        add_words(&mut words, notes_of(item), 1.0);
        for child in &children {
            add_words(&mut words, title_of(child), CHILD_WEIGHT);
        }
        let score = scorer.words(words)
            + scorer.labels(
                item.areas.iter().chain(children.iter().flat_map(|c| &c.areas)),
                item.repositories.iter().chain(children.iter().flat_map(|c| &c.repositories)),
            );
        if model.is_archived(&item.id) {
            if score > 0.0 {
                archived.push((item.id.clone(), score));
            }
        } else if !item.is_done() {
            let proximity = near.get(item.id.as_str()).copied().unwrap_or(0.0);
            parents.push((item.id.clone(), score + proximity));
        }
    }
    let threads = rank_threads(model, &scorer, &items);
    Ranking { parents: sorted(parents), threads: sorted(threads), archived: sorted(archived) }
}

/// Threads scored on their title, brief and members' titles and labels.
fn rank_threads(model: &Model, scorer: &Scorer<'_>, items: &[&Item]) -> Vec<(String, f64)> {
    let mut members: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
    for item in items {
        if let Some(thread) = item.thread.one() {
            members.entry(thread).or_default().push(item);
        }
    }
    let mut threads = Vec::new();
    for (id, thread) in &model.threads {
        let members = members.get(id.as_str()).map_or(&[][..], Vec::as_slice);
        // A thread whose work is all finished is no place for new work; one with no members yet is.
        if !members.is_empty() && members.iter().all(|m| m.is_done() || model.is_archived(&m.id)) {
            continue;
        }
        let mut words = Vector::new();
        add_words(&mut words, thread.title.one().map_or("", String::as_str), 1.0);
        add_words(&mut words, thread.brief.one().map_or("", String::as_str), 1.0);
        for member in members {
            add_words(&mut words, title_of(member), CHILD_WEIGHT);
        }
        let score = scorer.words(words)
            + scorer.labels(
                members.iter().flat_map(|m| &m.areas),
                thread.repositories.iter().chain(members.iter().flat_map(|m| &m.repositories)),
            );
        if score > 0.0 {
            threads.push((id.clone(), score));
        }
    }
    threads
}

/// The item and everything under it.
#[must_use]
pub fn subtree<'m>(model: &'m Model, id: &'m str) -> BTreeSet<&'m str> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![id];
    while let Some(next) = stack.pop() {
        if seen.insert(next) {
            stack.extend(model.children(next));
        }
    }
    seen
}

/// A parent candidate.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Parent {
    /// The item.
    pub item: String,
    /// Its title.
    pub title: String,
    /// Its score.
    pub score: f64,
    /// The thread the item would join under it, if any.
    pub inherits: Option<String>,
    /// The command that accepts it.
    pub accept: String,
    /// Said when accepting puts the item in approved scope, which an agent only proposes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'static str>,
}

/// A thread candidate.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThreadCandidate {
    /// The thread.
    pub thread: String,
    /// Its title.
    pub title: String,
    /// Its score.
    pub score: f64,
    /// The command that accepts it.
    pub accept: String,
    /// Said when the thread's scope is approved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'static str>,
}

/// An archived item that matches, offered as a link rather than a parent.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Related {
    /// The archived item.
    pub item: String,
    /// Its title.
    pub title: String,
    /// Its score.
    pub score: f64,
    /// The command that links to it.
    pub accept: String,
}

/// The candidates an add shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Suggestions {
    /// Live parents, best first.
    pub parents: Vec<Parent>,
    /// Threads, best first, for an item kept at the top level or under a parent with no thread.
    pub threads: Vec<ThreadCandidate>,
    /// Archived items that match better than a parent shown, as `relates` links.
    pub related: Vec<Related>,
}

const PROPOSED: &str = "approved scope: an agent's move is proposed to the user, not applied";

/// The candidates for item `id`, as a fresh add sees them, `limit` of each; and the rank of `given`, a parent the
/// caller named, among all parents, from 1, if it is among the first `limit`.
#[must_use]
pub fn suggest(
    model: &Model,
    id: &str,
    given: Option<&str>,
    limit: usize,
) -> (Suggestions, Option<usize>) {
    let Some(item) = model.items.get(id) else { return (Suggestions::default(), None) };
    let ranking = rank(model, &Query::of(model, item), &subtree(model, id));
    // Ranked among the parents that would be shown, which all share something with the item.
    let given_rank = given
        .and_then(|given| {
            let shown = ranking.parents.iter().filter(|(_, score)| *score > 0.0);
            shown.into_iter().position(|(parent, _)| parent == given)
        })
        .filter(|rank| *rank < limit)
        .map(|rank| rank + 1);
    let title = |id: &str| model.items.get(id).map(|i| title_of(i).to_owned()).unwrap_or_default();
    let approved =
        |thread: &str| model.threads.get(thread).is_some_and(crate::model::Thread::is_approved);
    let parents: Vec<Parent> = ranking
        .parents
        .iter()
        .filter(|(_, score)| *score > 0.0)
        .take(limit)
        .map(|(parent, score)| {
            let inherits = place::passed_down(model, parent).map(str::to_owned);
            Parent {
                item: parent.clone(),
                title: title(parent),
                score: *score,
                note: inherits.as_deref().filter(|t| approved(t)).map(|_| PROPOSED),
                inherits,
                accept: format!("acyclic item move {id} {parent}"),
            }
        })
        .collect();
    let weakest = if parents.len() < limit { 0.0 } else { parents.last().map_or(0.0, |p| p.score) };
    let related = ranking
        .archived
        .iter()
        .filter(|(_, score)| *score > weakest)
        .take(limit)
        .map(|(archived, score)| Related {
            item: archived.clone(),
            title: title(archived),
            score: *score,
            accept: format!("acyclic link add {id} relates {archived}"),
        })
        .collect();
    let threads = ranking
        .threads
        .iter()
        .take(limit)
        .map(|(thread, score)| ThreadCandidate {
            thread: thread.clone(),
            title: model
                .threads
                .get(thread)
                .and_then(|t| t.title.one().cloned())
                .unwrap_or_default(),
            score: *score,
            accept: format!("acyclic thread add {id} {thread}"),
            note: approved(thread).then_some(PROPOSED),
        })
        .collect();
    (Suggestions { parents, threads, related }, given_rank)
}

#[cfg(test)]
mod tests;
