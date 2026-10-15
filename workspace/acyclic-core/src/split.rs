//! Automatic splits: open leaves that no thread holds are grouped into draft threads, which agents refine
//! and the user approves. A split is computed, not judged: it keeps linked work together, keeps each draft inside one
//! subtree, and caps a draft's size. It never moves an item that a thread holds, so approved scope is never touched;
//! the one exception heals a conflict between unapproved split threads, which concurrent splits on two replicas make.
//!
//! The steps, over each top-level subtree (or the one root asked for):
//!
//! 1. The candidates are open, unarchived leaves with no thread. A subtree whose parent is in conflict waits until the
//!    conflict is resolved, since it belongs to two places at once.
//! 2. Candidates tied by a link that orders work (`blocks` or `after`), a link on a group counting for every leaf under
//!    it, form components. A component
//!    of at most [`CAP`] leaves stays whole; a larger one is cut, leaf by leaf, along the tree.
//! 3. Each piece sits at its leaves' lowest common ancestor. Bottom up, a node gathers its children's open groups and
//!    its own pieces: while they fit in [`CAP`] they stay one group, open to its parent; otherwise they are packed into
//!    as few drafts of even size as fit, rooted at that node.
//! 4. Drafts that `blocks` links would make wait on each other in a cycle are merged, which may exceed the cap or span
//!    two top-level items; the draft says so, since leaving them apart would deadlock their runs. A cycle through an
//!    existing thread can't be merged, since a split never touches one; the draft names that thread instead.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::Serialize;

use crate::model::{self, By, LinkKind, MAX_ID_LEN, Model, Thread, Value};
use crate::schema::{kind, thread};
use crate::scope::{self, ScopeEdit, ScopeError};
use crate::state::{Key, Replica};

/// The most leaves a draft holds, unless a cycle merged it.
pub const CAP: usize = 25;
/// Unassigned open leaves in a top-level subtree beyond which an add splits it.
pub const BUDGET: usize = 10;
/// The prefix of a split thread's id.
pub const PREFIX: &str = "s-";

/// A draft thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Draft {
    /// Its id: the prefix, its root's id and a hash of its members, so equal splits agree and different ones differ.
    pub id: String,
    /// The item whose subtree holds every member; for a merged cycle across top-level items, the first of them.
    pub root: String,
    /// A title for agents to improve: the root's, with an area or repository every member shares.
    pub title: String,
    /// Its members.
    pub members: BTreeSet<String>,
    /// Whether it merged drafts that would wait on each other, which may take it past the cap.
    pub merged_cycle: bool,
    /// Whether that merge spans top-level items, so `root` holds only some members.
    pub cross_top: bool,
    /// Existing threads it would wait on in a cycle, which only a change to those threads can break.
    pub waits_with: BTreeSet<String>,
}

/// Whether a split may place an item: it has no thread, or only conflicting unapproved split threads.
#[must_use]
pub fn unassigned(model: &Model, id: &str) -> bool {
    let Some(item) = model.items.get(id) else { return false };
    match &item.thread {
        Value::None => true,
        Value::One(_) => false,
        Value::Conflict(threads) => threads.iter().all(|thread| {
            thread.starts_with(PREFIX)
                && !model.threads.get(thread).is_some_and(Thread::is_approved)
        }),
    }
}

/// Whether an item is a candidate: an open, unarchived leaf that a split may place.
fn candidate(model: &Model, id: &str) -> bool {
    model.children(id).is_empty()
        && model.items.get(id).is_some_and(|item| !item.is_done() && !item.parent.is_conflict())
        && !model.is_archived(id)
        && unassigned(model, id)
}

/// The drafts for every top-level subtree, or for `root`'s subtree alone.
#[must_use]
pub fn plan(model: &Model, root: Option<&str>) -> Vec<Draft> {
    let tops: Vec<&str> = match root {
        Some(root) if model.items.contains_key(root) => vec![root],
        Some(_) => Vec::new(),
        None => model
            .items
            .values()
            .filter(|item| item.parent == Value::None && !model.is_archived(&item.id))
            .map(|item| item.id.as_str())
            .collect(),
    };
    let mut split = Split { model, under: BTreeMap::new(), top: BTreeMap::new() };
    for top in &tops {
        split.collect(top, top, &mut BTreeSet::new());
    }
    let pieces = split.pieces();
    let mut drafts = Vec::new();
    for top in &tops {
        let mut seen = BTreeSet::new();
        let (closed, open) = split.pack(top, &pieces, &mut seen);
        drafts.extend(closed);
        drafts.extend(open);
    }
    let drafts = split.merge_cycles(drafts);
    let waits = split.waits_with(&drafts);
    let mut taken: BTreeSet<String> = model.threads.keys().cloned().collect();
    drafts
        .into_iter()
        .zip(waits)
        .map(|((root, group, merged_cycle), waits_with)| {
            let tops: BTreeSet<&str> = group.iter().map(|leaf| split.top[leaf]).collect();
            let members: BTreeSet<String> = group.into_iter().map(str::to_owned).collect();
            // A taken id is a thread emptied since, or a collision: either way, never written into.
            let mut salt = 0;
            let mut id = draft_id(&root, &members, salt);
            while taken.contains(&id) {
                salt += 1;
                id = draft_id(&root, &members, salt);
            }
            taken.insert(id.clone());
            Draft {
                id,
                title: title(model, &root, &members),
                root,
                members,
                merged_cycle,
                cross_top: tops.len() > 1,
                waits_with,
            }
        })
        .collect()
}

struct Split<'a> {
    model: &'a Model,
    /// Each node's candidate leaves, its own included.
    under: BTreeMap<&'a str, Vec<&'a str>>,
    /// Each candidate's top-level item.
    top: BTreeMap<&'a str, &'a str>,
}

type Group<'a> = BTreeSet<&'a str>;
/// A group and the node it sits at.
type Rooted<'a> = (&'a str, Group<'a>);

/// What can wait on what across a split: a draft, by its index, or an existing thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Waiter<'a> {
    Draft(usize),
    Thread(&'a str),
}

impl<'a> Split<'a> {
    /// Records the candidates under `node`, guarding against a parent cycle a join made.
    fn collect(
        &mut self,
        node: &'a str,
        top: &'a str,
        seen: &mut BTreeSet<&'a str>,
    ) -> Vec<&'a str> {
        if !seen.insert(node) {
            return Vec::new();
        }
        let children = self.model.children(node);
        let leaves = if children.is_empty() {
            if candidate(self.model, node) {
                self.top.insert(node, top);
                vec![node]
            } else {
                Vec::new()
            }
        } else {
            self.settled_children(node)
                .into_iter()
                .flat_map(|child| self.collect(child, top, seen))
                .collect()
        };
        self.under.insert(node, leaves.clone());
        leaves
    }

    /// A node's children but those whose parent is in conflict, which sit under two nodes at once.
    fn settled_children(&self, node: &'a str) -> Vec<&'a str> {
        let model = self.model;
        model
            .children(node)
            .into_iter()
            .filter(|child| !model.items[*child].parent.is_conflict())
            .collect()
    }

    /// The candidates under a link's end.
    fn leaves(&self, id: &str) -> &[&'a str] {
        self.under.get(id).map_or(&[], Vec::as_slice)
    }

    /// The pieces, each placed at its leaves' lowest common ancestor.
    fn pieces(&self) -> BTreeMap<&'a str, Vec<Group<'a>>> {
        let leaves: Vec<&str> = self.top.keys().copied().collect();
        let index: BTreeMap<&str, usize> =
            leaves.iter().enumerate().map(|(i, leaf)| (*leaf, i)).collect();
        let mut parent: Vec<usize> = (0..leaves.len()).collect();
        for link in self.model.links.iter().filter(|link| link.kind.orders()) {
            let ends: Vec<&str> =
                self.leaves(&link.from).iter().chain(self.leaves(&link.to)).copied().collect();
            // Tie the ends within each top-level subtree; a draft never spans two.
            let mut first: BTreeMap<&str, usize> = BTreeMap::new();
            for leaf in ends {
                let i = index[leaf];
                match first.get(self.top[leaf]) {
                    Some(&j) => {
                        let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                        parent[a.max(b)] = a.min(b);
                    },
                    None => {
                        first.insert(self.top[leaf], i);
                    },
                }
            }
        }
        let mut components: BTreeMap<usize, Group<'a>> = BTreeMap::new();
        for (i, leaf) in leaves.iter().enumerate() {
            components.entry(find(&mut parent, i)).or_default().insert(leaf);
        }
        let mut pieces: BTreeMap<&str, Vec<Group<'a>>> = BTreeMap::new();
        for component in components.into_values() {
            if component.len() <= CAP {
                pieces.entry(self.lowest_common_ancestor(&component)).or_default().push(component);
            } else {
                for leaf in component {
                    pieces.entry(leaf).or_default().push(BTreeSet::from([leaf]));
                }
            }
        }
        pieces
    }

    fn lowest_common_ancestor(&self, leaves: &Group<'a>) -> &'a str {
        let mut leaves = leaves.iter();
        let first = leaves.next().expect("a component has a leaf");
        let lines: Vec<BTreeSet<&str>> =
            leaves.map(|leaf| self.line(leaf).into_iter().collect()).collect();
        self.line(first)
            .into_iter()
            .find(|node| lines.iter().all(|line| line.contains(node)))
            .unwrap_or(first)
    }

    /// An item and its ancestors, nearest first.
    fn line(&self, id: &'a str) -> Vec<&'a str> {
        std::iter::once(id).chain(self.model.ancestors(id)).collect()
    }

    /// The drafts closed in `node`'s subtree, and the groups still open to its parent, each with the node it sits at.
    fn pack(
        &self,
        node: &'a str,
        pieces: &BTreeMap<&'a str, Vec<Group<'a>>>,
        seen: &mut BTreeSet<&'a str>,
    ) -> (Vec<Rooted<'a>>, Vec<Rooted<'a>>) {
        if !seen.insert(node) {
            return (Vec::new(), Vec::new());
        }
        let mut closed = Vec::new();
        let mut groups: Vec<Rooted<'a>> = Vec::new();
        for child in self.settled_children(node) {
            let (child_closed, open) = self.pack(child, pieces, seen);
            closed.extend(child_closed);
            groups.extend(open);
        }
        groups.extend(pieces.get(node).into_iter().flatten().map(|piece| (node, piece.clone())));
        let total: usize = groups.iter().map(|(_, group)| group.len()).sum();
        if total <= CAP {
            let group: Group<'a> = groups.into_iter().flat_map(|(_, group)| group).collect();
            return (closed, if group.is_empty() { Vec::new() } else { vec![(node, group)] });
        }
        // Like with like: the first area, then the first repository, then the id, decide the order of first fit. Bins
        // aim at an even share of the fewest that could hold everything, so 26 leaves make 13 and 13, not 25 and 1. A
        // bin of one group keeps that group's own root, so a child's whole subtree stays a draft of its own.
        groups.sort_by_cached_key(|(_, group)| self.affinity(group));
        let fewest = total.div_ceil(CAP);
        let even = total.div_ceil(fewest);
        let mut bins: Vec<Rooted<'a>> = Vec::new();
        for (root, group) in groups {
            let size = group.len();
            let fits = |limit: usize| move |(_, bin): &Rooted<'a>| bin.len() + size <= limit;
            // An even share first, then a new bin while there are fewer than the fewest, and only then up to the cap.
            let bin = bins.iter().position(fits(even)).or_else(|| {
                (bins.len() >= fewest).then(|| bins.iter().position(fits(CAP))).flatten()
            });
            match bin {
                Some(i) => {
                    bins[i].0 = node;
                    bins[i].1.extend(group);
                },
                None => bins.push((root, group)),
            }
        }
        closed.extend(bins);
        (closed, Vec::new())
    }

    fn affinity(&self, group: &Group<'a>) -> (String, String, String) {
        let first = group.first().expect("a group has a leaf");
        let item = &self.model.items[*first];
        (
            item.areas.first().cloned().unwrap_or_default(),
            item.repositories.first().cloned().unwrap_or_default(),
            (*first).to_owned(),
        )
    }

    /// Merges drafts that `blocks` links make wait on each other in a cycle.
    fn merge_cycles(&self, drafts: Vec<Rooted<'a>>) -> Vec<(String, Group<'a>, bool)> {
        let mut draft_of: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, (_, members)) in drafts.iter().enumerate() {
            for member in members {
                draft_of.insert(member, i);
            }
        }
        let drafts_under = |id: &str| -> BTreeSet<usize> {
            self.leaves(id).iter().filter_map(|leaf| draft_of.get(leaf).copied()).collect()
        };
        let mut edges = Vec::new();
        for link in self.model.links.iter().filter(|link| link.kind == LinkKind::Blocks) {
            for to in drafts_under(&link.to) {
                for from in drafts_under(&link.from) {
                    if from != to {
                        edges.push((to, from));
                    }
                }
            }
        }
        let mut merged_into: BTreeMap<usize, usize> = BTreeMap::new();
        for cycle in crate::graph::cycles(&edges) {
            for &i in &cycle[1..] {
                merged_into.insert(i, cycle[0]);
            }
        }
        let mut out: BTreeMap<usize, (Vec<&str>, Group<'a>, bool)> = BTreeMap::new();
        for (i, (root, members)) in drafts.into_iter().enumerate() {
            let target = merged_into.get(&i).copied().unwrap_or(i);
            let entry = out.entry(target).or_default();
            entry.0.push(root);
            entry.1.extend(members);
            entry.2 |= target != i || merged_into.values().any(|&t| t == i);
        }
        out.into_values()
            .map(|(roots, members, merged)| {
                let root = if roots.len() == 1 { roots[0] } else { self.common_root(&roots) };
                (root.to_owned(), members, merged)
            })
            .collect()
    }

    /// For each draft, the existing threads it would wait on in a cycle through `blocks` links.
    fn waits_with(&self, drafts: &[(String, Group<'a>, bool)]) -> Vec<BTreeSet<String>> {
        let mut draft_of: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, (_, members, _)) in drafts.iter().enumerate() {
            for member in members {
                draft_of.insert(member, i);
            }
        }
        let mut out = vec![BTreeSet::new(); drafts.len()];
        for cycle in self.wait_cycles(&draft_of) {
            let threads: BTreeSet<String> = cycle
                .iter()
                .filter_map(|node| match node {
                    Waiter::Thread(thread) => Some((*thread).to_owned()),
                    Waiter::Draft(_) => None,
                })
                .collect();
            for node in &cycle {
                if let Waiter::Draft(i) = node {
                    out[*i].extend(threads.iter().cloned());
                }
            }
        }
        out
    }

    /// The cycles that `blocks` links make among drafts, by `draft_of`, and existing threads.
    fn wait_cycles(&self, draft_of: &BTreeMap<&str, usize>) -> Vec<Vec<Waiter<'a>>> {
        let mut memo = BTreeMap::new();
        let mut edges = Vec::new();
        for link in self.model.links.iter().filter(|link| link.kind == LinkKind::Blocks) {
            let to = self.waiters(&link.to, draft_of, &mut memo);
            let from = self.waiters(&link.from, draft_of, &mut memo);
            for to in &to {
                for from in from.iter().filter(|from| *from != to) {
                    edges.push((*to, *from));
                }
            }
        }
        crate::graph::cycles(&edges)
    }

    /// The drafts and threads that hold open items under `id`, it included.
    fn waiters(
        &self,
        id: &'a str,
        draft_of: &BTreeMap<&str, usize>,
        memo: &mut BTreeMap<&'a str, BTreeSet<Waiter<'a>>>,
    ) -> BTreeSet<Waiter<'a>> {
        if let Some(known) = memo.get(id) {
            return known.clone();
        }
        let mut found = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            if !seen.insert(node) {
                continue;
            }
            if let Some(&i) = draft_of.get(node) {
                found.insert(Waiter::Draft(i));
            } else if let Some(thread) = self.model.thread_of(node)
                && !self.model.items[node].is_done()
            {
                found.insert(Waiter::Thread(thread));
            }
            stack.extend(self.model.children(node));
        }
        memo.insert(id, found.clone());
        found
    }

    /// The nearest item whose subtree holds every root, or the first root when none does.
    fn common_root(&self, roots: &[&'a str]) -> &'a str {
        let lines: Vec<BTreeSet<&str>> =
            roots[1..].iter().map(|root| self.line(root).into_iter().collect()).collect();
        self.line(roots[0])
            .into_iter()
            .find(|node| lines.iter().all(|line| line.contains(node)))
            .unwrap_or(roots[0])
    }
}

/// The representative of `i`'s set in a union-find forest, halving paths on the way.
fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// The existing threads that `blocks` links make wait on each other, one set per cycle: work in them can't all
/// finish until a link or a member moves.
#[must_use]
pub fn thread_cycles(model: &Model) -> Vec<BTreeSet<String>> {
    let split = Split { model, under: BTreeMap::new(), top: BTreeMap::new() };
    split
        .wait_cycles(&BTreeMap::new())
        .into_iter()
        .map(|cycle| {
            cycle
                .into_iter()
                .filter_map(|node| match node {
                    Waiter::Thread(thread) => Some(thread.to_owned()),
                    Waiter::Draft(_) => None,
                })
                .collect()
        })
        .collect()
}

/// Splits as [`plan`] does and writes the drafts: each a new thread with its title, holding its members. A draft is
/// only ever written into a thread id no thread has, so no existing thread, approved or not, gains members.
///
/// # Errors
///
/// Fails when the state layer refuses.
pub fn apply(
    replica: &mut Replica,
    root: Option<&str>,
    by: By<'_>,
) -> Result<Vec<Draft>, ScopeError> {
    let model = Model::read(replica.state())?;
    let drafts = plan(&model, root);
    for draft in &drafts {
        let key = Key::new(kind::THREAD, &draft.id)?;
        model::put(replica, &key, thread::TITLE, Some(&draft.title), by)?;
        for member in &draft.members {
            let change = ScopeEdit::Member { item: member.clone(), thread: Some(draft.id.clone()) };
            scope::edit_in(replica, &model, &change, by)?;
        }
    }
    Ok(drafts)
}

/// `s-<root>-<hash>`: the root's id, cut to fit, and 24 bits of an FNV-1a hash of the sorted members, salted when an
/// unsalted id is taken.
fn draft_id(root: &str, members: &BTreeSet<String>, salt: u32) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let salt = (salt > 0).then(|| salt.to_string());
    for member in members.iter().chain(salt.as_ref()) {
        for byte in member.bytes().chain(std::iter::once(b'\n')) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    let fits = MAX_ID_LEN - PREFIX.len() - 7;
    let root = &root[..root.len().min(fits)];
    let mut id = format!("{PREFIX}{root}-");
    write!(id, "{:06x}", hash & 0xff_ffff).expect("a String takes writes");
    id
}

/// The root's title, with an area or else a repository that every member shares.
fn title(model: &Model, root: &str, members: &BTreeSet<String>) -> String {
    let base = model
        .items
        .get(root)
        .and_then(|item| item.title.one().cloned())
        .unwrap_or_else(|| root.to_owned());
    let shared = |field: fn(&crate::model::Item) -> &BTreeSet<String>| -> Option<String> {
        let mut sets = members.iter().filter_map(|id| model.items.get(id)).map(field);
        let first = sets.next()?.clone();
        sets.fold(first, |acc, set| acc.intersection(set).cloned().collect()).into_iter().next()
    };
    match shared(|item| &item.areas).or_else(|| shared(|item| &item.repositories)) {
        Some(tag) => format!("{base} · {tag}"),
        None => base,
    }
}

#[cfg(test)]
mod tests;
