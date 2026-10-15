//! Automatic splits: open leaves that no thread holds are grouped into draft threads, which agents refine and the user
//! approves. A split is computed, not judged: it keeps linked work together, keeps each draft inside one
//! subtree, and caps a draft's size. It never moves an item that a thread holds, so approved scope is never touched;
//! the one exception heals a conflict between unapproved split threads, which concurrent splits on two replicas make.
//!
//! The steps, over each top-level subtree (or the one root asked for):
//!
//! 1. The candidates are open, unarchived leaves with no thread. A subtree whose parent is in conflict waits until the
//!    conflict is resolved, since it belongs to two places at once.
//! 2. Candidates tied by a link that orders work (`blocks` or `after`), a link on a group counting for every leaf under
//!    it, form components. A component of at most [`CAP`] leaves stays whole as a piece; a larger one is cut into
//!    single leaves.
//! 3. Each piece sits at its leaves' lowest common ancestor. Bottom up, a node gathers its children's open groups and
//!    its own pieces: while they fit in [`CAP`] they stay one group, open to its parent; otherwise they are packed into
//!    as few drafts of even size as fit, rooted at that node.
//! 4. Drafts that `blocks` links would make wait on each other in a cycle, which would deadlock their runs, are cut
//!    again along an order every `blocks` link among them follows, so they wait one way only. Only pieces that wait on
//!    each other in a cycle must share a draft, which may then pass the cap or span two top-level items; the draft says
//!    so. A cycle through an existing thread can't be cut, since a split never touches one; the draft names that
//!    thread instead.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::fmt::Write as _;

use serde::Serialize;

use crate::model::{self, By, LinkKind, MAX_ID_LEN, Model, Thread, Value};
use crate::schema::{kind, thread};
use crate::scope::{self, ScopeEdit, ScopeError};
use crate::state::{Error, Key, Replica, State};

/// The most leaves a draft holds, unless pieces waiting on each other must share it.
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
    /// The item whose subtree holds every member; for a draft across top-level items, the first of them.
    pub root: String,
    /// A title for agents to improve: the root's, with an area or repository every member shares.
    pub title: String,
    /// Its members.
    pub members: BTreeSet<String>,
    /// Whether it holds leaves that wait on each other in a cycle, which may take it past the cap.
    pub merged_cycle: bool,
    /// Whether it spans top-level items, so `root` holds only some members.
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

/// No items held back.
static NONE: BTreeSet<String> = BTreeSet::new();

/// The drafts for every top-level subtree, or for `root`'s subtree alone.
#[must_use]
pub fn plan(model: &Model, root: Option<&str>) -> Vec<Draft> {
    plan_with(model, root, &NONE)
}

/// Like [`plan`], leaving out the `held` items, such as those [`held`] names.
#[must_use]
pub fn plan_with(model: &Model, root: Option<&str>, held: &BTreeSet<String>) -> Vec<Draft> {
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
    plan_tops(model, &tops, held)
}

/// The items a pending proposal would put in a thread, or take out of one: the user decides where they go, so a
/// split leaves them be.
///
/// # Errors
///
/// Fails on an unreadable proposal.
pub fn held(state: &State) -> Result<BTreeSet<String>, Error> {
    Ok(scope::proposals(state)?
        .into_values()
        .filter(|proposal| proposal.decision == Value::None)
        .filter_map(|proposal| match proposal.edit.one() {
            Some(ScopeEdit::Member { item, .. }) => Some(item.clone()),
            _ => None,
        })
        .collect())
}

/// The drafts for these top-level items' subtrees.
fn plan_tops(model: &Model, tops: &[&str], held: &BTreeSet<String>) -> Vec<Draft> {
    let mut split = Split::new(model, held);
    for top in tops {
        split.collect(top, top, &mut BTreeSet::new());
    }
    let (pieces, unit_of) = split.pieces();
    let mut drafts = Vec::new();
    for top in tops {
        let mut seen = BTreeSet::new();
        let (closed, open) = split.pack(top, &pieces, &mut seen);
        drafts.extend(closed);
        drafts.extend(open);
    }
    let drafts = split.untangle(drafts, &unit_of);
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
    /// Each candidate's place in a walk of the tree, which orders work that links leave unordered.
    position: BTreeMap<&'a str, usize>,
    /// Items that would be candidates but wait on a proposal.
    held: &'a BTreeSet<String>,
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
    fn new(model: &'a Model, held: &'a BTreeSet<String>) -> Self {
        let (under, top, position) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        Self { model, under, top, position, held }
    }

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
            if candidate(self.model, node) && !self.held.contains(node) {
                self.top.insert(node, top);
                self.position.insert(node, self.position.len());
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

    /// The pieces, each placed at its leaves' lowest common ancestor, and the piece each candidate is in.
    fn pieces(&self) -> (BTreeMap<&'a str, Vec<Group<'a>>>, BTreeMap<&'a str, usize>) {
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
        let mut unit_of = BTreeMap::new();
        let mut add = |at: &'a str, piece: Group<'a>| {
            let unit = unit_of.len();
            unit_of.extend(piece.iter().map(|leaf| (*leaf, unit)));
            pieces.entry(at).or_default().push(piece);
        };
        for component in components.into_values() {
            if component.len() <= CAP {
                add(self.lowest_common_ancestor(&component), component);
            } else {
                for leaf in component {
                    add(leaf, BTreeSet::from([leaf]));
                }
            }
        }
        (pieces, unit_of)
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

    /// Untangles drafts that `blocks` links make wait on each other in a cycle: their leaves are cut again, in an
    /// order that every `blocks` link among them follows, into even drafts that wait on each other only one way.
    /// Pieces stay whole unless they wait on each other in a cycle, and then only leaves that do so themselves must
    /// share a draft, which may then pass the cap.
    fn untangle(
        &self,
        drafts: Vec<Rooted<'a>>,
        unit_of: &BTreeMap<&'a str, usize>,
    ) -> Vec<(String, Group<'a>, bool)> {
        let mut draft_of: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, (_, members)) in drafts.iter().enumerate() {
            for member in members {
                draft_of.insert(member, i);
            }
        }
        let edges = self.blocks_edges(|leaf| draft_of.get(leaf).copied());
        let cycles = crate::graph::cycles(&edges);
        let tangled: BTreeSet<usize> = cycles.iter().flatten().copied().collect();
        let mut drafts: Vec<Option<Rooted<'a>>> = drafts.into_iter().map(Some).collect();
        let mut out = Vec::new();
        for (i, draft) in drafts.iter_mut().enumerate() {
            if !tangled.contains(&i)
                && let Some((root, members)) = draft.take()
            {
                out.push((root.to_owned(), members, false));
            }
        }
        for cycle in cycles {
            let leaves: Group<'a> = cycle
                .iter()
                .filter_map(|&i| drafts[i].take())
                .flat_map(|(_, members)| members)
                .collect();
            out.extend(self.recut(&leaves, unit_of));
        }
        out
    }

    /// The `(depended on, dependent)` pairs that `blocks` links make between the nodes `node_of` maps leaves to.
    fn blocks_edges(&self, node_of: impl Fn(&str) -> Option<usize>) -> Vec<(usize, usize)> {
        let mut edges = BTreeSet::new();
        for link in self.model.links.iter().filter(|link| link.kind == LinkKind::Blocks) {
            let nodes = |id: &str| -> BTreeSet<usize> {
                self.leaves(id).iter().filter_map(|leaf| node_of(leaf)).collect()
            };
            let from = nodes(&link.from);
            for to in nodes(&link.to) {
                edges.extend(from.iter().filter(|from| **from != to).map(|from| (to, *from)));
            }
        }
        edges.into_iter().collect()
    }

    /// Cuts `leaves` into even drafts along an order of their pieces that every `blocks` link among them follows,
    /// earliest in the tree first. Pieces that wait on each other in a cycle give way to their leaves, and only leaves
    /// that wait on each other in a cycle become one.
    fn recut(
        &self,
        leaves: &Group<'a>,
        unit_of: &BTreeMap<&'a str, usize>,
    ) -> Vec<(String, Group<'a>, bool)> {
        let pieces = self.blocks_edges(|leaf: &str| leaves.contains(leaf).then(|| unit_of[leaf]));
        let tangled: BTreeSet<usize> =
            crate::graph::cycles(&pieces).into_iter().flatten().collect();
        // A tangled piece's leaves become units of their own, numbered past every piece.
        let unit_of: BTreeMap<&'a str, usize> = leaves
            .iter()
            .enumerate()
            .map(|(i, leaf)| {
                let unit = unit_of[leaf];
                (*leaf, if tangled.contains(&unit) { unit_of.len() + i } else { unit })
            })
            .collect();
        let unit_of = &unit_of;
        let unit = |leaf: &str| leaves.contains(leaf).then(|| unit_of[leaf]);
        let edges = self.blocks_edges(unit);
        let mut node: BTreeMap<usize, usize> =
            leaves.iter().map(|leaf| (unit_of[leaf], unit_of[leaf])).collect();
        let mut merged = BTreeSet::new();
        for cycle in crate::graph::cycles(&edges) {
            for &u in &cycle {
                node.insert(u, cycle[0]);
            }
            merged.insert(cycle[0]);
        }
        let mut nodes: BTreeMap<usize, Group<'a>> = BTreeMap::new();
        for leaf in leaves {
            nodes.entry(node[&unit_of[leaf]]).or_default().insert(leaf);
        }
        let mut waiting: BTreeMap<usize, usize> = nodes.keys().map(|&n| (n, 0)).collect();
        let mut next: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for (to, from) in edges {
            let (to, from) = (node[&to], node[&from]);
            if to != from && next.entry(to).or_default().insert(from) {
                *waiting.get_mut(&from).expect("a node") += 1;
            }
        }
        let first = |n: usize| nodes[&n].iter().map(|leaf| self.position[leaf]).min().unwrap_or(0);
        let mut ready: BinaryHeap<Reverse<(usize, usize)>> = waiting
            .iter()
            .filter(|(_, w)| **w == 0)
            .map(|(&n, _)| Reverse((first(n), n)))
            .collect();
        let mut order = Vec::with_capacity(nodes.len());
        while let Some(Reverse((_, n))) = ready.pop() {
            order.push(n);
            for &m in next.get(&n).into_iter().flatten() {
                let w = waiting.get_mut(&m).expect("a node");
                *w -= 1;
                if *w == 0 {
                    ready.push(Reverse((first(m), m)));
                }
            }
        }
        let even = leaves.len().div_ceil(leaves.len().div_ceil(CAP));
        let mut chunks: Vec<(Group<'a>, bool)> = Vec::new();
        for n in order {
            let group = nodes.remove(&n).expect("each node once");
            match chunks.last_mut() {
                Some((chunk, merged_cycle))
                    if chunk.len() < even && chunk.len() + group.len() <= CAP =>
                {
                    chunk.extend(group);
                    *merged_cycle |= merged.contains(&n);
                },
                _ => chunks.push((group, merged.contains(&n))),
            }
        }
        chunks
            .into_iter()
            .map(|(chunk, merged_cycle)| {
                let tops: BTreeSet<&str> = chunk.iter().map(|leaf| self.top[leaf]).collect();
                let root = match tops.first() {
                    Some(top) if tops.len() > 1 => top,
                    _ => self.lowest_common_ancestor(&chunk),
                };
                (root.to_owned(), chunk, merged_cycle)
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
    let split = Split::new(model, &NONE);
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
    let drafts = plan_with(&model, root, &held(replica.state())?);
    write(replica, &model, &drafts, by)?;
    Ok(drafts)
}

fn write(
    replica: &mut Replica,
    model: &Model,
    drafts: &[Draft],
    by: By<'_>,
) -> Result<(), ScopeError> {
    for draft in drafts {
        let key = Key::new(kind::THREAD, &draft.id)?;
        model::put(replica, &key, thread::TITLE, Some(&draft.title), by)?;
        for member in &draft.members {
            let change = ScopeEdit::Member { item: member.clone(), thread: Some(draft.id.clone()) };
            scope::edit_in(replica, model, &change, by)?;
        }
    }
    Ok(())
}

/// The thread new work under `id`'s parent joins, as [`passed_down`] says; `None` under a parent in conflict.
#[must_use]
pub fn inherited_thread<'m>(model: &'m Model, id: &str) -> Option<&'m str> {
    inheriting(model, id, &BTreeMap::new())
}

fn inheriting<'m>(model: &'m Model, id: &str, joined: &BTreeMap<&str, usize>) -> Option<&'m str> {
    // A parent in conflict has no single line to inherit along.
    let parent = model.items.get(id).and_then(|item| item.parent.one())?;
    passing(model, parent, joined)
}

/// The thread a new child of `parent` joins: `parent`'s own; else the one its threaded children share, while that
/// holds fewer than [`CAP`] open members, so a group's later children join the draft its first child started; else
/// its nearest threaded ancestor's. `None` when the nearest thread on that line is in conflict, or there is none.
#[must_use]
pub fn passed_down<'m>(model: &'m Model, parent: &str) -> Option<&'m str> {
    passing(model, parent, &BTreeMap::new())
}

/// [`passed_down`], counting `joined` new members toward each thread's cap.
fn passing<'m>(model: &'m Model, parent: &str, joined: &BTreeMap<&str, usize>) -> Option<&'m str> {
    if model.items.get(parent).is_some_and(|item| item.thread == Value::None)
        && let Some(shared) = shared_by_children(model, parent, joined)
    {
        return Some(shared);
    }
    std::iter::once(parent)
        .chain(model.ancestors(parent))
        .filter_map(|id| model.items.get(id))
        .map(|item| &item.thread)
        .find(|thread| **thread != Value::None)
        .and_then(Value::one)
        .map(String::as_str)
}

/// The one thread that `parent`'s open, unarchived children with a thread all hold, while it has room; `None` when
/// any of them is in conflict or they differ. Finished work doesn't pull new work into its thread.
fn shared_by_children<'m>(
    model: &'m Model,
    parent: &str,
    joined: &BTreeMap<&str, usize>,
) -> Option<&'m str> {
    let open = |id: &str| {
        model.items.get(id).is_some_and(|item| !item.is_done()) && !model.is_archived(id)
    };
    let mut shared = None;
    for child in model.children(parent).into_iter().filter(|child| open(child)) {
        match &model.items[child].thread {
            Value::None => {},
            Value::One(thread) if shared.is_none_or(|shared| shared == thread.as_str()) => {
                shared = Some(thread.as_str());
            },
            _ => return None,
        }
    }
    let shared = shared?;
    let members = model.members(shared).into_iter().filter(|id| open(id)).count();
    (members + joined.get(shared).copied().unwrap_or(0) < CAP).then_some(shared)
}

/// An item that joined its nearest threaded ancestor's thread, or asked to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Inherited {
    /// The item.
    pub item: String,
    /// The thread.
    pub thread: String,
    /// The proposal it waits as, when the thread's scope is approved and the actor may only propose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
}

/// What placing new or moved work did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Placement {
    /// Items that joined a thread from above.
    pub inherited: Vec<Inherited>,
    /// Draft threads a split wrote.
    pub drafts: Vec<Draft>,
}

impl Placement {
    /// Whether it did nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inherited.is_empty() && self.drafts.is_empty()
    }
}

/// Places `items` after an add, a move or an import. An item with no thread joins the one [`passed_down`] names,
/// through the scope rules, so an agent only proposes it into approved scope; an item that has a thread keeps it.
/// Then each top-level subtree the items are in is split when it holds more than `budget` free leaves, when an item
/// is its parent's first child, so the parent just became a group, or always without a budget.
///
/// # Errors
///
/// Fails when the state layer refuses.
pub fn place(
    replica: &mut Replica,
    items: &[String],
    budget: Option<usize>,
    by: By<'_>,
) -> Result<Placement, ScopeError> {
    Ok(placing(replica, items, budget, by)?.0)
}

/// Like [`place`], also returning the model as it stands after placing: the one placing read when it wrote no draft,
/// so a caller that reads next saves a read.
///
/// # Errors
///
/// Fails when the state layer refuses.
pub fn place_and_read(
    replica: &mut Replica,
    items: &[String],
    budget: Option<usize>,
    by: By<'_>,
) -> Result<(Placement, Model), ScopeError> {
    let (placement, model) = placing(replica, items, budget, by)?;
    let model = if placement.drafts.is_empty() { model } else { Model::read(replica.state())? };
    Ok((placement, model))
}

/// Places, returning the model read before any draft was written.
fn placing(
    replica: &mut Replica,
    items: &[String],
    budget: Option<usize>,
    by: By<'_>,
) -> Result<(Placement, Model), ScopeError> {
    let mut placement = Placement::default();
    let mut model = Model::read(replica.state())?;
    let mut joined: BTreeMap<&str, usize> = BTreeMap::new();
    for item in items {
        let Some(found) = model.items.get(item) else { continue };
        if found.thread != Value::None || found.is_done() {
            continue;
        }
        if let Some(thread) = inheriting(&model, item, &joined) {
            *joined.entry(thread).or_default() += 1;
            let change = ScopeEdit::Member { item: item.clone(), thread: Some(thread.to_owned()) };
            let proposal = match scope::edit_in(replica, &model, &change, by)? {
                scope::Outcome::Applied => None,
                scope::Outcome::Proposed(id) => Some(id),
            };
            placement.inherited.push(Inherited {
                item: item.clone(),
                thread: thread.to_owned(),
                proposal,
            });
        }
    }
    // An applied join writes only the item's thread, which was unset: set it here rather than read the state again.
    for joined in placement.inherited.iter().filter(|joined| joined.proposal.is_none()) {
        if let Some(item) = model.items.get_mut(&joined.item) {
            item.thread = Value::One(joined.thread.clone());
        }
    }
    let model = model;
    let tops: BTreeSet<&str> = items
        .iter()
        .filter(|item| model.items.contains_key(item.as_str()))
        .map(|item| model.ancestors(item).last().copied().unwrap_or(item))
        .filter(|top| model.items[*top].parent == Value::None && !model.is_archived(top))
        .collect();
    let held = held(replica.state())?;
    let top_of = |item: &str| model.ancestors(item).last().copied().unwrap_or(item).to_owned();
    let sprouted: BTreeSet<String> = items
        .iter()
        .filter(|item| {
            let parent = model.items.get(item.as_str()).and_then(|i| i.parent.one());
            parent.is_some_and(|parent| model.children(parent) == [item.as_str()])
        })
        .map(|item| top_of(item))
        .collect();
    let over: Vec<&str> = tops
        .into_iter()
        .filter(|top| {
            let mut split = Split::new(&model, &held);
            split.collect(top, top, &mut BTreeSet::new());
            budget.is_none_or(|budget| split.top.len() > budget) || sprouted.contains(*top)
        })
        .collect();
    placement.drafts = plan_tops(&model, &over, &held);
    write(replica, &model, &placement.drafts, by)?;
    Ok((placement, model))
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
