//! Shape statistics of a plan, for checking generated fixtures against their target shape.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use acyclic_core::import::{Import, State};

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
    /// Threads whose repository set has several repositories.
    pub multi_repository_threads: usize,
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
        let (mut cross_root_threads, mut multi_repository_threads) = (0, 0);
        let mut thread_sizes = Vec::new();
        for thread in &import.threads {
            let members: Vec<usize> = (0..import.items.len())
                .filter(|&i| import.items[i].thread.as_ref() == Some(&thread.id))
                .collect();
            thread_sizes.push(members.len());
            threaded.extend(members.iter().copied());
            let roots: BTreeSet<usize> = members.iter().map(|&i| root(i).0).collect();
            let repositories: BTreeSet<&str> = members
                .iter()
                .flat_map(|&i| import.items[i].repositories.iter().flatten().map(String::as_str))
                .chain(thread.repositories.iter().flatten().map(String::as_str))
                .collect();
            cross_root_threads += usize::from(roots.len() > 1);
            multi_repository_threads += usize::from(repositories.len() > 1);
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
            multi_repository_threads,
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
