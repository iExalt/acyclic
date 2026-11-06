#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic, missing_docs)]
// Shapes are rates and counts; converting between them is the point.
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]

mod rng;
pub mod stats;

use std::collections::{BTreeSet, HashMap, HashSet};

use acyclic_core::import::{Evidence, Import, Item, Repository, State, Thread};

pub use rng::SplitMix64;

/// The item count the shape's rates are measured at.
pub const BASE_ITEMS: usize = 350;

/// The clock fixture ids are made at: 2026-01-01T00:00:00Z, a millisecond later for each record, so ids sort in the
/// order the records were made.
const CLOCK: u64 = 1_767_225_600_000;

/// e^-1.3, for a Poisson draw of 1.3 hard links from each item below the top level, before those that would repeat or
/// break a rule are skipped. A constant rather than `exp`, whose last bit varies by platform.
const EXP_MINUS_1_3: f64 = 0.272_531_793_034_012_6;

/// Areas the generator still draws, so each seed keeps its plan's structure, though plans no longer carry them.
const AREAS: [&str; 7] = ["core", "api", "storage", "interface", "infra", "research", "docs"];
const VERBS: [&str; 12] = [
    "Add", "Build", "Measure", "Split", "Wire", "Document", "Harden", "Port", "Profile", "Design",
    "Test", "Replace",
];
const NOUNS: [&str; 16] = [
    "parser",
    "scheduler",
    "cache",
    "index",
    "renderer",
    "importer",
    "exporter",
    "planner",
    "queue",
    "ledger",
    "worker",
    "gateway",
    "registry",
    "catalog",
    "monitor",
    "loader",
];
const QUALIFIERS: [&str; 10] = [
    "for batches",
    "with retries",
    "behind a flag",
    "on startup",
    "under load",
    "for replicas",
    "end to end",
    "at scale",
    "in the app",
    "from the CLI",
];

/// Generates a plan of exactly `items` items (at least 50) from `seed`.
///
/// # Panics
///
/// Panics when `items` is below 50, too few to hold the shape.
#[must_use]
pub fn generate(items: usize, seed: u64) -> Import {
    assert!(items >= 50, "a plan needs at least 50 items");
    Generator::new(items, seed).run()
}

struct Generator {
    rng: SplitMix64,
    /// The ids' random bits, apart from `rng`, so a plan's structure doesn't depend on how many ids it makes.
    ids: SplitMix64,
    /// Records made so far, each a millisecond after the last.
    made: u64,
    /// Each item's id.
    item_ids: Vec<String>,
    /// Each repository's id: the product, deploy and website repositories.
    repository_ids: Vec<String>,
    /// Each item's repository, by index into `repository_ids`: the one its evidence names, if it is done.
    repository: Vec<Option<usize>>,
    total: usize,
    scale: f64,
    import: Import,
    /// Each item's depth, 1 for roots.
    depth: Vec<usize>,
    /// Each item's root index.
    root: Vec<usize>,
    /// Each item's parent index.
    parent: Vec<Option<usize>>,
}

impl Generator {
    fn new(total: usize, seed: u64) -> Self {
        let scale = total as f64 / BASE_ITEMS as f64;
        Self {
            rng: SplitMix64::new(seed),
            ids: SplitMix64::new(seed ^ 0x6964_735f_7365_6564),
            made: 0,
            item_ids: Vec::new(),
            repository_ids: Vec::new(),
            repository: Vec::new(),
            total,
            scale,
            import: Import::new(),
            depth: Vec::new(),
            root: Vec::new(),
            parent: Vec::new(),
        }
    }

    fn scaled(&self, at_base: f64) -> usize {
        let n = (at_base * self.scale).round() as usize;
        n.max(1)
    }

    fn run(mut self) -> Import {
        self.repositories();
        self.tree();
        self.states();
        let threads = self.threads();
        self.links(&threads);
        self.import
    }

    /// A new `UUIDv7` from the seed, a millisecond after the last.
    fn id(&mut self) -> String {
        let random = self.ids.next_u64().to_le_bytes();
        let more = self.ids.next_u64().to_le_bytes();
        let mut bytes = [0; 10];
        bytes[..8].copy_from_slice(&random);
        bytes[8..].copy_from_slice(&more[..2]);
        self.made += 1;
        acyclic_core::ids::at(CLOCK + self.made, bytes)
    }

    fn repositories(&mut self) {
        for name in ["product", "deploy", "website"] {
            let id = self.id();
            self.repository_ids.push(id.clone());
            self.import.repositories.push(Repository {
                id,
                name: format!("example/{name}"),
                default_branch: "main".to_owned(),
            });
        }
    }

    fn title(&mut self) -> String {
        let verb = VERBS[self.rng.below(VERBS.len())];
        let noun = NOUNS[self.rng.below(NOUNS.len())];
        let qualifier = QUALIFIERS[self.rng.below(QUALIFIERS.len())];
        format!("{verb} the {noun} {qualifier}")
    }

    fn push_item(&mut self, parent: Option<usize>) -> usize {
        let index = self.import.items.len();
        let (depth, root) = match parent {
            Some(p) => (self.depth[p] + 1, self.root[p]),
            None => (1, index),
        };
        let title = self.title();
        let roll = self.rng.unit();
        let repository = if roll < 0.85 {
            0
        } else if roll < 0.95 {
            1
        } else {
            2
        };
        // A size draw and a resource draw plans no longer carry, kept so each seed keeps its structure.
        if depth >= 3 {
            self.rng.below(6);
        }
        let _ = depth >= 3 && self.rng.chance(0.01);
        let id = self.id();
        self.item_ids.push(id.clone());
        self.import.items.push(Item {
            id,
            title,
            notes: self.rng.chance(0.3).then(|| "Synthetic design notes.".to_owned()),
            state: Some(State::Backlog),
            parent: parent.map(|p| self.item_ids[p].clone()),
            thread: None,
            waits_for: None,
            evidence: None,
        });
        self.repository.push((depth > 1).then_some(repository));
        self.depth.push(depth);
        self.root.push(root);
        self.parent.push(parent);
        index
    }

    /// Roots, then umbrellas under roots, then leaves under umbrellas, and a few sub-items under leaves. Items are
    /// laid out depth first, so an item's index orders it after everything it may depend on.
    fn tree(&mut self) {
        let roots = self.scaled(2.0).max(2);
        let umbrellas = self.scaled(28.0);
        let leaves = self.total - roots - umbrellas;
        // Umbrellas per root, then leaves per umbrella, as near-even shares with jitter.
        let per_root = self.shares(umbrellas, roots);
        let mut umbrella_leaves = self.shares(leaves, umbrellas).into_iter();
        for umbrella_count in per_root {
            let root = self.push_item(None);
            for _ in 0..umbrella_count {
                // The areas' draws, kept so each seed keeps its structure.
                let _ = self.rng.below(AREAS.len());
                let umbrella = self.push_item(Some(root));
                let mut last_leaf = None;
                for _ in 0..umbrella_leaves.next().unwrap_or(0) {
                    if !self.rng.chance(0.005) && !self.rng.chance(0.85) {
                        let _ = self.rng.below(AREAS.len());
                    }
                    // About one leaf in twenty is a sub-item of the previous leaf.
                    let parent = match last_leaf {
                        Some(leaf) if self.depth[leaf] == 3 && self.rng.chance(0.05) => leaf,
                        _ => umbrella,
                    };
                    let index = self.push_item(Some(parent));
                    if self.depth[index] == 3 {
                        last_leaf = Some(index);
                    }
                }
            }
        }
        debug_assert_eq!(self.import.items.len(), self.total);
    }

    /// Splits `total` into `parts` shares of about equal size, each at least 1 when possible.
    fn shares(&mut self, total: usize, parts: usize) -> Vec<usize> {
        let weights: Vec<f64> = (0..parts).map(|_| 0.5 + self.rng.unit()).collect();
        apportion(total, &weights, 1, total)
    }

    /// Done work is a prefix of the depth-first order, so every dependency of a done item is done too; only leaves are
    /// marked done, since a parent reads done from its leaves.
    ///
    /// The prefix and the random draws over it are as they were when umbrellas were stored done too, so the rest of
    /// the fixture, which the seed's threads and links come from, stays as it was.
    fn states(&mut self) {
        let done_target = self.scaled(35.0);
        let mut prefix = vec![false; self.total];
        let mut done = 0;
        let mut index = 0;
        while done < done_target && index < self.total {
            if self.depth[index] >= 3 {
                prefix[index] = true;
                done += 1;
            }
            index += 1;
        }
        let prefix_end = index;
        // An umbrella in the prefix once read done when its whole subtree lay inside the prefix.
        for umbrella in (0..prefix_end).filter(|&i| self.depth[i] == 2) {
            let subtree_end =
                (umbrella + 1..self.total).find(|&i| self.depth[i] <= 2).unwrap_or(self.total);
            prefix[umbrella] = subtree_end <= prefix_end;
        }
        let parents: HashSet<usize> = self.parent.iter().flatten().copied().collect();
        for (index, &in_prefix) in prefix.iter().enumerate() {
            let leaf = !parents.contains(&index);
            let item = &mut self.import.items[index];
            if in_prefix {
                // Drawn for parents too, which keeps the draws after them where they were.
                if self.rng.chance(0.5) {
                    let (a, b, c) = (self.rng.next_u64(), self.rng.next_u64(), self.rng.next_u64());
                    let sha = format!("{a:016x}{b:016x}{:08x}", c >> 32);
                    // Only items under a root are done, and each has a repository.
                    let repository = self.repository[index].map(|r| &self.repository_ids[r]);
                    item.evidence = leaf.then(|| Evidence {
                        commit: repository.map(|repository| format!("{repository}@{sha}")),
                        text: None,
                    });
                }
                if leaf {
                    item.state = Some(State::Done);
                }
            } else if self.depth[index] >= 3 {
                // A share between 0.04 and 0.05 once drew a stored in-progress state; it stays open.
                if self.rng.unit() < 0.04 {
                    item.state = Some(State::Parked);
                }
            }
        }
    }

    /// Threads take contiguous runs of non-root items in depth-first order, so most stay within one umbrella; a few
    /// swaps make some cross roots. About an eighth of the items stay unthreaded.
    fn threads(&mut self) -> Vec<Option<usize>> {
        let count = self.scaled(21.0);
        let small = self.scaled(6.0).min(count - 1);
        let big = count - small;
        let eligible: Vec<usize> = (0..self.total).filter(|&i| self.depth[i] >= 2).collect();
        let unthreaded_target = (self.total as f64 * 0.125).round() as usize;
        let roots = self.total - eligible.len();
        let threaded = eligible.len() - unthreaded_target.saturating_sub(roots).min(eligible.len());

        let small_sizes: Vec<usize> = (0..small).map(|_| 1 + self.rng.below(2)).collect();
        let big_total = threaded - small_sizes.iter().sum::<usize>();
        // Skewed toward small threads, with the first pinned near the top of the range.
        let mut weights: Vec<f64> = (0..big)
            .map(|_| {
                let r = self.rng.unit();
                0.2 + r * r
            })
            .collect();
        weights[0] = 3.0;
        let mut sizes = apportion(big_total, &weights, 4, 46);
        sizes.extend(small_sizes);
        self.rng.shuffle(&mut sizes);

        // Leave the unthreaded share out at random, then cut the rest into runs.
        let mut pool = eligible.clone();
        let mut skipped = HashSet::new();
        while pool.len() > threaded {
            let at = self.rng.below(pool.len());
            skipped.insert(pool.remove(at));
        }
        let mut owner = vec![None; self.total];
        let mut members: Vec<Vec<usize>> = Vec::with_capacity(count);
        let mut cursor = 0;
        for (thread, &size) in sizes.iter().enumerate() {
            let run: Vec<usize> = pool[cursor..cursor + size].to_vec();
            for &item in &run {
                owner[item] = Some(thread);
            }
            members.push(run);
            cursor += size;
        }
        // Swap a sixtieth of the members between threads, including one swap across roots.
        let swaps = (threaded / 60).max(1);
        for n in 0..swaps {
            let a = pool[self.rng.below(pool.len())];
            let b = if n == 0 {
                let other = pool.iter().copied().filter(|&i| self.root[i] != self.root[a]);
                let candidates: Vec<usize> = other.collect();
                candidates[self.rng.below(candidates.len())]
            } else {
                pool[self.rng.below(pool.len())]
            };
            let (ta, tb) = (owner[a].unwrap(), owner[b].unwrap());
            if ta != tb {
                members[ta].retain(|&i| i != a);
                members[tb].retain(|&i| i != b);
                members[ta].push(b);
                members[tb].push(a);
                owner.swap(a, b);
            }
        }

        for run in &mut members {
            run.sort_unstable();
            let title = self.title();
            let id = self.id();
            for &member in run.iter() {
                self.import.items[member].thread = Some(id.clone());
            }
            self.import.threads.push(Thread {
                id,
                title,
                brief: Some("A synthetic thread.".to_owned()),
            });
        }
        owner
    }

    /// Whether an item reads done: a leaf stored done, or a parent whose leaves all are. A subtree follows its root
    /// in the depth-first order, deeper than it.
    fn reads_done(&self, index: usize) -> bool {
        let subtree = (index + 1..self.total).take_while(|&i| self.depth[i] > self.depth[index]);
        let mut leaves =
            subtree.filter(|&i| self.depth.get(i + 1).is_none_or(|&next| next <= self.depth[i]));
        if self.depth.get(index + 1).is_some_and(|&next| next > self.depth[index]) {
            leaves.all(|i| self.import.items[i].state == Some(State::Done))
        } else {
            self.import.items[index].state == Some(State::Done)
        }
    }

    fn is_related_by_ancestry(&self, a: usize, b: usize) -> bool {
        let ancestor_of = |upper: usize, mut lower: usize| {
            while let Some(p) = self.parent[lower] {
                if p == upper {
                    return true;
                }
                lower = p;
            }
            false
        };
        ancestor_of(a, b) || ancestor_of(b, a)
    }

    /// Waits-for links average about 1.2 per item, always from a later item to an earlier one, so they never form a
    /// cycle. No link joins an item to its ancestor or descendant. The related links once drawn at 0.05 per item are
    /// still drawn, so each seed keeps its structure, and written nowhere.
    fn links(&mut self, owner: &[Option<usize>]) {
        let mut by_thread: HashMap<usize, Vec<usize>> = HashMap::new();
        for (item, thread) in owner.iter().enumerate() {
            if let Some(thread) = thread {
                by_thread.entry(*thread).or_default().push(item);
            }
        }
        let mut seen = BTreeSet::new();
        let candidates: Vec<usize> = (0..self.total).filter(|&i| self.depth[i] >= 2).collect();
        for &from in &candidates {
            let hard = self.rng.poisson(EXP_MINUS_1_3);
            let related = usize::from(self.rng.chance(0.05));
            for (waits, n) in [(true, hard), (false, related)] {
                for _ in 0..n {
                    let pool: &[usize] = match owner[from] {
                        Some(thread) if self.rng.chance(0.75) => &by_thread[&thread],
                        _ => &candidates,
                    };
                    let earlier: Vec<usize> = pool
                        .iter()
                        .copied()
                        .filter(|&to| to < from && !self.is_related_by_ancestry(from, to))
                        .collect();
                    if earlier.is_empty() {
                        continue;
                    }
                    let to = earlier[self.rng.below(earlier.len())];
                    // Done items depend only on done items; skip links that would break that.
                    if self.import.items[from].state == Some(State::Done) && !self.reads_done(to) {
                        continue;
                    }
                    if !seen.insert((from, to, waits)) || !waits {
                        continue;
                    }
                    let to = self.item_ids[to].clone();
                    self.import.items[from].waits_for.get_or_insert_with(Vec::new).push(to);
                }
            }
        }
    }
}

/// Splits `total` in proportion to `weights`, each share within `min..=max`, summing exactly to `total` when the
/// bounds allow.
fn apportion(total: usize, weights: &[f64], min: usize, max: usize) -> Vec<usize> {
    let sum: f64 = weights.iter().sum();
    let mut shares: Vec<usize> = weights
        .iter()
        .map(|w| ((w / sum) * total as f64).floor() as usize)
        .map(|s| s.clamp(min, max))
        .collect();
    let mut current: usize = shares.iter().sum();
    let mut i = 0;
    let mut stalled = 0;
    while current != total && stalled < shares.len() {
        let at = i % shares.len();
        if current < total && shares[at] < max {
            shares[at] += 1;
            current += 1;
            stalled = 0;
        } else if current > total && shares[at] > min {
            shares[at] -= 1;
            current -= 1;
            stalled = 0;
        } else {
            stalled += 1;
        }
        i += 1;
    }
    shares
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::stats::Stats;

    fn check_shape(items: usize, seed: u64) -> Stats {
        let import = generate(items, seed);
        assert_eq!(import.validate(), vec![], "{items}/{seed} validates");
        let stats = Stats::of(&import);
        let at = format!("{items} items, seed {seed}: {stats:#?}");
        let scale = items as f64 / BASE_ITEMS as f64;
        let scaled = |n: f64| (n * scale).round() as usize;

        assert_eq!(stats.items, items, "{at}");
        assert!(
            (1..=4).all(|d| stats.depths.get(&d).copied().unwrap_or(0) > 0),
            "every depth to 4: {at}"
        );
        assert_eq!(stats.depths.keys().max(), Some(&4), "{at}");

        assert_eq!(stats.thread_sizes.len(), scaled(21.0), "{at}");
        let big = stats.thread_sizes.iter().filter(|&&s| s >= 4).count();
        assert_eq!(big, scaled(15.0), "{at}");
        assert!(
            stats.thread_sizes.iter().all(|&s| (1..=2).contains(&s) || (4..=46).contains(&s)),
            "{at}"
        );
        assert!(stats.thread_sizes.iter().any(|&s| s >= 40), "a thread of 40 or more: {at}");
        let unthreaded = stats.unthreaded as f64 / items as f64;
        assert!((0.085..=0.165).contains(&unthreaded), "unthreaded share {unthreaded}: {at}");

        assert!(stats.cross_root_threads > 0, "{at}");

        assert!((0.07..=0.14).contains(&stats.state_share(State::Done)), "{at}");
        assert!((0.02..=0.06).contains(&stats.state_share(State::Parked)), "{at}");

        let near =
            |rate: f64, target: f64, tolerance: f64| (rate - target).abs() <= target * tolerance;
        assert!(near(stats.link_rate(), 1.2, 0.3), "{at}");
        stats
    }

    fn mean_thread_size(stats: &Stats) -> f64 {
        stats.thread_sizes.iter().sum::<usize>() as f64 / stats.thread_sizes.len() as f64
    }

    #[test]
    fn small_plans_match_the_shape() {
        for seed in 1..=3 {
            check_shape(BASE_ITEMS, seed);
        }
    }

    #[test]
    fn large_plans_scale_the_thread_count() {
        let small = mean_thread_size(&check_shape(BASE_ITEMS, 1));
        let large = mean_thread_size(&check_shape(10 * BASE_ITEMS, 1));
        assert!((large - small).abs() <= small * 0.1, "mean thread size {large} vs {small}");
    }

    /// FNV-1a, to pin a generated plan's exact bytes across platforms and releases.
    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    #[test]
    fn a_seed_gives_the_same_plan_everywhere() {
        // A deliberate change to the generator or the import format updates this digest; anything else changing it is a bug.
        let json = generate(BASE_ITEMS, 1).to_json();
        assert_eq!(generate(BASE_ITEMS, 1).to_json(), json, "the same seed twice");
        assert_eq!((json.len(), fnv1a(json.as_bytes())), (121_368, 8_288_387_897_255_007_585));
    }

    #[test]
    fn ids_sort_in_the_order_records_were_made() {
        let plan = generate(BASE_ITEMS, 1);
        let ids: Vec<&String> = plan.items.iter().map(|item| &item.id).collect();
        assert!(ids.is_sorted(), "items in the order they were made");
        assert!(plan.repositories.last().unwrap().id < *ids[0], "repositories first");
        assert!(ids.iter().all(|id| acyclic_core::ids::v7(id).as_deref() == Some(id.as_str())));
    }

    #[test]
    fn different_seeds_give_different_plans() {
        assert_ne!(generate(BASE_ITEMS, 9).to_json(), generate(BASE_ITEMS, 10).to_json());
    }

    #[test]
    fn apportion_sums_and_respects_bounds() {
        let shares = apportion(100, &[1.0, 1.0, 3.0], 4, 46);
        assert_eq!(shares.iter().sum::<usize>(), 100);
        assert!(shares.iter().all(|&s| (4..=46).contains(&s)));
        assert_eq!(apportion(7, &[1.0, 2.0], 1, 7).iter().sum::<usize>(), 7);
    }
}
