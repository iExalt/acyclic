//! Shape statistics of a plan, for checking generated fixtures against their target shape.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use acyclic_core::import::{Import, LinkKind, State};

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
    /// Distinct areas.
    pub areas: usize,
    /// Items with no area, roots excepted.
    pub without_area: usize,
    /// Threads whose items span several areas.
    pub cross_area_threads: usize,
    /// Threads whose items span several roots.
    pub cross_goal_threads: usize,
    /// Threads whose repository set has several repositories.
    pub multi_repository_threads: usize,
    /// Items per state.
    pub states: BTreeMap<String, usize>,
    /// Archived items.
    pub archived: usize,
    /// Links per kind.
    pub links: BTreeMap<String, usize>,
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
            import.items.iter().enumerate().map(|(i, item)| (item.key.as_str(), i)).collect();
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
        let (mut cross_area_threads, mut cross_goal_threads, mut multi_repository_threads) =
            (0, 0, 0);
        for thread in &import.threads {
            let members: Vec<usize> = thread.items.iter().map(|k| index[k.as_str()]).collect();
            threaded.extend(members.iter().copied());
            let areas: BTreeSet<&str> = members
                .iter()
                .flat_map(|&i| import.items[i].areas.iter().map(String::as_str))
                .collect();
            let goals: BTreeSet<usize> = members.iter().map(|&i| root(i).0).collect();
            let repositories: BTreeSet<&str> = members
                .iter()
                .flat_map(|&i| import.items[i].repositories.iter().map(String::as_str))
                .chain(thread.repositories.iter().map(String::as_str))
                .collect();
            cross_area_threads += usize::from(areas.len() > 1);
            cross_goal_threads += usize::from(goals.len() > 1);
            multi_repository_threads += usize::from(repositories.len() > 1);
        }
        let mut states = BTreeMap::new();
        for item in &import.items {
            *states.entry(format!("{:?}", item.state)).or_default() += 1;
        }
        let mut links = BTreeMap::new();
        for link in &import.links {
            *links.entry(format!("{:?}", link.kind)).or_default() += 1;
        }
        Self {
            items: import.items.len(),
            depths,
            thread_sizes: import.threads.iter().map(|t| t.items.len()).collect(),
            unthreaded: import.items.len() - threaded.len(),
            areas: import.items.iter().flat_map(|i| &i.areas).collect::<BTreeSet<_>>().len(),
            without_area: import
                .items
                .iter()
                .filter(|i| i.parent.is_some() && i.areas.is_empty())
                .count(),
            cross_area_threads,
            cross_goal_threads,
            multi_repository_threads,
            states,
            archived: import.items.iter().filter(|i| i.archived).count(),
            links,
        }
    }

    /// The share of items in a state.
    #[must_use]
    pub fn state_share(&self, state: State) -> f64 {
        self.states.get(&format!("{state:?}")).copied().unwrap_or(0) as f64 / self.items as f64
    }

    /// Links of a kind per item.
    #[must_use]
    pub fn link_rate(&self, kind: LinkKind) -> f64 {
        self.links.get(&format!("{kind:?}")).copied().unwrap_or(0) as f64 / self.items as f64
    }
}
