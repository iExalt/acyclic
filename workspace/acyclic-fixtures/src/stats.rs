//! Shape statistics of a plan, for checking generated fixtures against their target shape.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use acyclic_core::import::{Import, State};
use acyclic_core::model::{By, Model};
use acyclic_core::ready::ReadinessEvaluator;
use acyclic_core::scope::USER;
use acyclic_core::state::{Replica, ReplicaId, State as Doc};

/// A plan's shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    /// Items.
    pub items: usize,
    /// Items per depth, 1 for roots.
    pub depths: BTreeMap<usize, usize>,
    /// Each thread's size.
    pub thread_sizes: Vec<usize>,
    /// Items in no thread.
    pub unthreaded: usize,
    /// Threads whose items span several roots.
    pub cross_root_threads: usize,
    /// Items per state.
    pub states: BTreeMap<String, usize>,
    /// Waits-for links.
    pub links: usize,
}

impl Stats {
    /// Measures a plan. Its references must resolve.
    ///
    /// # Panics
    ///
    /// Panics when a reference doesn't resolve.
    #[must_use]
    pub fn of(import: &Import) -> Self {
        let index: HashMap<&str, usize> =
            import.items.iter().enumerate().map(|(i, item)| (item.id.as_str(), i)).collect();
        let parent: Vec<Option<usize>> =
            import.items.iter().map(|item| item.parent.as_deref().map(|p| index[p])).collect();
        let root = |mut i: usize| {
            let mut depth = 1;
            while let Some(p) = parent[i] {
                i = p;
                depth += 1;
            }
            (i, depth)
        };

        let mut depths = BTreeMap::new();
        for i in 0..import.items.len() {
            *depths.entry(root(i).1).or_default() += 1;
        }
        let mut threaded = BTreeSet::new();
        let mut cross_root_threads = 0;
        let mut thread_sizes = Vec::new();
        for thread in &import.threads {
            let members: Vec<usize> = (0..import.items.len())
                .filter(|&i| import.items[i].thread.as_ref() == Some(&thread.id))
                .collect();
            thread_sizes.push(members.len());
            threaded.extend(members.iter().copied());
            let roots: BTreeSet<usize> = members.iter().map(|&i| root(i).0).collect();
            cross_root_threads += usize::from(roots.len() > 1);
        }
        let mut states = BTreeMap::new();
        for item in &import.items {
            *states.entry(format!("{:?}", item.state.unwrap_or(State::Backlog))).or_default() += 1;
        }
        Self {
            items: import.items.len(),
            depths,
            thread_sizes,
            unthreaded: import.items.len() - threaded.len(),
            cross_root_threads,
            states,
            links: import.items.iter().map(|i| i.waits_for.as_ref().map_or(0, Vec::len)).sum(),
        }
    }

    /// The share of items in a state.
    #[must_use]
    pub fn state_share(&self, state: State) -> f64 {
        self.states.get(&format!("{state:?}")).copied().unwrap_or(0) as f64 / self.items as f64
    }

    /// Waits-for links per item.
    #[must_use]
    pub fn link_rate(&self) -> f64 {
        self.links as f64 / self.items as f64
    }
}

/// The sizes of the plan's thread cycles, as the tracker reads them: groups of threads that wait on each other, through
/// their items' links and their ancestors', with work left.
///
/// # Panics
///
/// Panics when the plan doesn't apply to an empty tracker.
#[must_use]
pub fn thread_cycles(import: &Import) -> Vec<usize> {
    let mut replica = Replica::open(ReplicaId::new("fixtures").unwrap(), Doc::new());
    acyclic_core::import::apply(import, &mut replica, By { actor: USER, at: 1 }).unwrap();
    let model = Model::read(replica.state()).unwrap();
    let readiness = ReadinessEvaluator::new(replica.state(), &model, 2);
    let mut sizes: Vec<usize> =
        readiness.thread_cycles().unwrap().iter().map(std::collections::BTreeSet::len).collect();
    sizes.sort_unstable();
    sizes
}
