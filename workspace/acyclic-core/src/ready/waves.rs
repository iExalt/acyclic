//! Threads in waves: which to start first, and which after. A thread waits on another when one of its open leaves
//! waits on unfinished work in it, through the item's own links or its ancestors', as readiness reads them; so work
//! done but not yet merged still holds back the threads waiting on it, as it holds back their leaves.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{Readiness, Stage};
use crate::graph;
use crate::natural;
use crate::state::Error;

/// The threads with work left, in waves: the first holds those nothing holds back, and each thread comes after every
/// thread it waits on, unless they wait on each other, when they share a wave. Threads that wait on none and that
/// none waits on can start any time, and stand apart. Ids are in natural order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Waves {
    /// The waves, first to last.
    pub waves: Vec<Vec<String>>,
    /// The threads in no wave.
    pub anytime: Vec<String>,
    /// The threads each thread waits on, for every thread with work left.
    pub waits_for: BTreeMap<String, Vec<String>>,
}

/// The threads with work left, and each wait between two of them, as (waited on, waiting).
type ThreadWaits<'m> = (BTreeSet<&'m str>, Vec<(&'m str, &'m str)>);

impl Readiness<'_> {
    /// The threads with work left, in waves.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn waves(&self) -> Result<Waves, Error> {
        let (left, edges) = self.thread_waits()?;
        let mut nodes: Vec<&str> = left.iter().copied().collect();
        nodes.sort_by(|a, b| natural::cmp(a, b));
        let mut waits_for: BTreeMap<String, Vec<String>> =
            nodes.iter().map(|&thread| (thread.to_owned(), Vec::new())).collect();
        for &(from, to) in &edges {
            waits_for.entry(to.to_owned()).or_default().push(from.to_owned());
        }
        for list in waits_for.values_mut() {
            list.sort_by(|a, b| natural::cmp(a, b));
        }
        let (waves, anytime) = graph::waves(&nodes, &edges);
        let owned = |ids: Vec<&str>| ids.into_iter().map(str::to_owned).collect::<Vec<_>>();
        Ok(Waves {
            waves: waves.into_iter().map(owned).collect(),
            anytime: owned(anytime),
            waits_for,
        })
    }

    /// The threads that wait on each other, one set per cycle, as the waves read waits: work in them can't all finish
    /// until a link or a member moves. Such threads share a wave.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn thread_cycles(&self) -> Result<Vec<BTreeSet<String>>, Error> {
        let (_, edges) = self.thread_waits()?;
        Ok(graph::cycles(&edges)
            .into_iter()
            .map(|cycle| cycle.into_iter().map(str::to_owned).collect())
            .collect())
    }

    /// The threads with work left, and each wait between two of them.
    fn thread_waits(&self) -> Result<ThreadWaits<'_>, Error> {
        let model = self.model;
        let mut left: BTreeSet<&str> = BTreeSet::new();
        let mut edges: BTreeSet<(&str, &str)> = BTreeSet::new();
        // Many leaves wait on one dependency, such as all those under a parent with a link: each is walked once.
        let mut under: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
        for (id, item) in &model.items {
            let Some(thread) = self.recorded_thread(id) else { continue };
            if !self.has_work_left(id)? {
                continue;
            }
            left.insert(thread);
            // Done work waits on nothing, and parked work waits for the user, not for other threads.
            if item.is_done() || self.stage(id)? == Some(Stage::Parked) {
                continue;
            }
            for dependency in self.waiting(id).into_keys() {
                if !under.contains_key(&dependency) {
                    let found = self.unfinished_under(&dependency)?;
                    under.insert(dependency.clone(), found);
                }
                for &blocker in &under[&dependency] {
                    if blocker != thread {
                        edges.insert((blocker, thread));
                    }
                }
            }
        }
        // Work waited on in a thread with none left of its own can't hold anything back.
        let edges = edges
            .into_iter()
            .filter(|(from, to)| left.contains(from) && left.contains(to))
            .collect();
        Ok((left, edges))
    }

    /// An item's thread, when the plan has a record of it.
    fn recorded_thread(&self, id: &str) -> Option<&str> {
        self.model.thread_of(id).filter(|thread| self.model.threads.contains_key(*thread))
    }

    /// Whether `id` is a leaf with work left: any stage but done, so work in review counts.
    fn has_work_left(&self, id: &str) -> Result<bool, Error> {
        Ok(self.stage(id)?.is_some_and(|stage| stage != Stage::Done))
    }

    /// The threads of the unfinished leaves at or under `id`.
    fn unfinished_under(&self, id: &str) -> Result<BTreeSet<&str>, Error> {
        let mut found = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut stack = vec![id];
        while let Some(at) = stack.pop() {
            if !seen.insert(at) {
                continue;
            }
            if let Some(thread) = self.recorded_thread(at)
                && self.has_work_left(at)?
            {
                found.insert(thread);
            }
            stack.extend(self.model.children(at));
        }
        Ok(found)
    }
}
