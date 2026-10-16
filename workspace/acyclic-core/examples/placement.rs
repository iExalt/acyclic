//! How often placement candidates find an item's true parent: each item that has a parent is held out in turn (it
//! and its subtree leave the candidates and the word weights) and ranked against the rest of a plan.
//!
//! `cargo run --release -p acyclic-core --example placement -- <import.json>`
//!
//! Two settings: "add" ranks on the item's title and labels alone, as a fresh `acyclic item add` sees it; "with
//! links" adds its notes and its own links, as a move or an import sees it. Each rate stands beside its chance level, the
//! expected rate under a random order of the same candidates. Only aggregates are printed, never an id or a title,
//! so the plan's contents stay private.

use std::collections::BTreeMap;
use std::process::ExitCode;

use acyclic_core::import::{self, Import};
use acyclic_core::model::{By, Model};
use acyclic_core::scope::USER;
use acyclic_core::state::{Replica, ReplicaId, State};
use acyclic_core::suggest::{self, Query};

/// Sums over held-out items.
#[derive(Default)]
struct Tally {
    items: usize,
    hit1: f64,
    hit3: f64,
    reciprocal: f64,
    chance1: f64,
    chance3: f64,
    chance_reciprocal: f64,
}

impl Tally {
    #[expect(clippy::cast_precision_loss, reason = "counts are small")]
    fn add(&mut self, rank: Option<usize>, candidates: usize) {
        self.items += 1;
        if let Some(rank) = rank {
            self.hit1 += f64::from(u8::from(rank == 1));
            self.hit3 += f64::from(u8::from(rank <= 3));
            self.reciprocal += 1.0 / rank as f64;
        }
        if candidates > 0 {
            let n = candidates as f64;
            self.chance1 += 1.0 / n;
            self.chance3 += (3.0 / n).min(1.0);
            self.chance_reciprocal += (1..=candidates).map(|k| 1.0 / k as f64).sum::<f64>() / n;
        }
    }

    #[expect(clippy::cast_precision_loss, reason = "counts are small")]
    fn print(&self, label: &str) {
        let n = self.items.max(1) as f64;
        println!(
            "  {label:<10} {:>5}   hit@1 {:.2} ({:.2})   hit@3 {:.2} ({:.2})   MRR {:.2} ({:.2})",
            self.items,
            self.hit1 / n,
            self.chance1 / n,
            self.hit3 / n,
            self.chance3 / n,
            self.reciprocal / n,
            self.chance_reciprocal / n,
        );
    }
}

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: placement <import.json>");
        return ExitCode::from(2);
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("the file can't be read");
        return ExitCode::from(1);
    };
    let doc = match Import::from_json(&text) {
        Ok(doc) => doc,
        Err(_) => {
            eprintln!("the file isn't an import document");
            return ExitCode::from(1);
        },
    };
    let mut replica = Replica::open(ReplicaId::new("eval").expect("a valid id"), State::new());
    if let Err(err) = import::apply(&doc, &mut replica, By { actor: USER, at: 1 }) {
        // The error's own text can name items; its kind can't.
        let kind = match err {
            import::ApplyError::Invalid(errors) => format!("{} validation error(s)", errors.len()),
            import::ApplyError::Cycles(cycles) => format!("{} cycle(s)", cycles.len()),
            import::ApplyError::UserOnly(_) => "a user-only record".to_owned(),
            import::ApplyError::Write(_) => "a refused write".to_owned(),
        };
        eprintln!("the import was refused: {kind}");
        return ExitCode::from(1);
    }
    let model = Model::read(replica.state()).expect("an imported state reads");
    println!(
        "{} items; constants: child weight {}, label bonus {}, proximity {}, leaf weight {}, floor {}",
        model.items.len(),
        suggest::CHILD_WEIGHT,
        suggest::LABEL_BONUS,
        suggest::PROXIMITY,
        suggest::LEAF_WEIGHT,
        suggest::MIN_SCORE
    );
    for (setting, with_links) in [("add", false), ("with links", true)] {
        let mut overall = Tally::default();
        let mut by_depth: BTreeMap<usize, Tally> = BTreeMap::new();
        let mut groups_chance3 = 0.0;
        for item in model.items.values() {
            let Some(parent) = item.parent.one() else { continue };
            let query = if with_links {
                Query::of(&model, item)
            } else {
                Query { notes: "", ..Query::words_of(item) }
            };
            let ranking = suggest::rank(&model, &query, &suggest::subtree(&model, &item.id));
            // A true parent scoring below the floor isn't found, wherever it sorts: an add shows only candidates
            // that reach it.
            let rank = ranking
                .parents
                .iter()
                .position(|(id, score)| id == parent && *score >= suggest::MIN_SCORE)
                .map(|i| i + 1);
            let candidates = ranking.parents.len();
            overall.add(rank, candidates);
            let groups =
                ranking.parents.iter().filter(|(id, _)| !model.children(id).is_empty()).count();
            if groups > 0 {
                #[expect(clippy::cast_precision_loss, reason = "counts are small")]
                let chance = (3.0 / groups as f64).min(1.0);
                groups_chance3 += chance;
            }
            by_depth.entry(model.ancestors(&item.id).len()).or_default().add(rank, candidates);
        }
        println!("{setting}: items, then each rate with its chance level in brackets");
        overall.print("all");
        for (depth, tally) in &by_depth {
            tally.print(&format!("depth {depth}"));
        }
        // Most parents are groups already, so chance among groups alone is the harder reference.
        #[expect(clippy::cast_precision_loss, reason = "counts are small")]
        let groups_chance3 = groups_chance3 / overall.items.max(1) as f64;
        println!("  hit@3 by chance among items with children only: {groups_chance3:.2}");
    }
    ExitCode::SUCCESS
}
