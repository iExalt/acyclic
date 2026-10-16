//! The graph as documents: a scoped view of items, their tree and links, with threads as clusters, written as DOT or
//! Mermaid. Every id and title is escaped for its format, since titles come from any actor.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::model::{LinkKind, Model, Value};

/// Which part of the graph to draw.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scope<'a> {
    /// Only this item and its descendants.
    pub root: Option<&'a str>,
    /// Only this thread's members; with `root`, the items in both.
    pub thread: Option<&'a str>,
    /// Draw archived items too.
    pub include_archived: bool,
}

/// An edge's kind: the tree, or a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeKind {
    /// Parent to child.
    Tree,
    /// A link of this kind.
    Link(LinkKind),
}

/// An edge, drawn from `from` to `to`: parent to child, or for a link, from what is depended on to the dependent, so
/// layouts flow in the order work happens.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    /// Where it starts.
    pub from: String,
    /// Where it ends.
    pub to: String,
    /// Its kind.
    pub kind: EdgeKind,
}

/// A scoped view of the graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    /// The items in scope.
    pub items: BTreeSet<String>,
    /// Items outside the scope that a link ties to an item in it, drawn one hop out without their own ties.
    pub ghosts: BTreeSet<String>,
    /// The edges between drawn nodes, sorted.
    pub edges: Vec<Edge>,
    /// Each thread with members in scope, and those members; items in no thread sit outside every cluster.
    pub clusters: BTreeMap<String, BTreeSet<String>>,
}

/// Why a view can't be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    /// The root isn't an item.
    NoItem(String),
    /// The thread doesn't exist.
    NoThread(String),
}

impl std::fmt::Display for ScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoItem(id) => write!(f, "no item {id}"),
            Self::NoThread(id) => write!(f, "no thread {id}"),
        }
    }
}

impl std::error::Error for ScopeError {}

/// The view of `model` that `scope` selects.
///
/// # Errors
///
/// Fails when the scope names an item or thread that doesn't exist.
pub fn view(model: &Model, scope: &Scope<'_>) -> Result<View, ScopeError> {
    if let Some(root) = scope.root
        && !model.items.contains_key(root)
    {
        return Err(ScopeError::NoItem(root.to_owned()));
    }
    if let Some(thread) = scope.thread
        && !model.threads.contains_key(thread)
    {
        return Err(ScopeError::NoThread(thread.to_owned()));
    }
    let in_root =
        |id: &str| scope.root.is_none_or(|root| id == root || model.ancestors(id).contains(&root));
    let items: BTreeSet<String> = model
        .items
        .keys()
        .filter(|id| scope.include_archived || !model.is_archived(id))
        .filter(|id| in_root(id))
        .filter(|id| scope.thread.is_none_or(|thread| model.thread_of(id) == Some(thread)))
        .cloned()
        .collect();
    let mut edges = Vec::new();
    let mut ghosts = BTreeSet::new();
    for id in &items {
        if let Some(parent) = model.items[id].parent.one()
            && items.contains(parent)
        {
            edges.push(Edge { from: parent.clone(), to: id.clone(), kind: EdgeKind::Tree });
        }
    }
    for link in &model.links {
        let (from, to) = (link.to.clone(), link.from.clone());
        match (items.contains(&from), items.contains(&to)) {
            (false, false) => continue,
            (true, false) => ghosts.insert(to.clone()),
            (false, true) => ghosts.insert(from.clone()),
            (true, true) => false,
        };
        edges.push(Edge { from, to, kind: EdgeKind::Link(link.kind) });
    }
    edges.sort();
    edges.dedup();
    let mut clusters: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for id in &items {
        if let Some(thread) = model.thread_of(id) {
            clusters.entry(thread.to_owned()).or_default().insert(id.clone());
        }
    }
    Ok(View { items, ghosts, edges, clusters })
}

/// A node or cluster id safe in both formats: letters and digits as they are, and every other byte as `_` and its
/// hex, so distinct ids stay distinct and none is a keyword.
fn node_id(prefix: char, id: &str) -> String {
    let mut out = String::from(prefix);
    out.push('_');
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() {
            out.push(char::from(byte));
        } else {
            write!(out, "_{byte:02x}").expect("a String takes writes");
        }
    }
    out
}

/// A file name for a scope's page, distinct per scope: `all`, or `root-…` and `thread-…` with ids encoded as node
/// ids are.
#[must_use]
pub fn scope_name(scope: &Scope<'_>) -> String {
    let encode = |kind: &str, id: &str| format!("{kind}-{}", &node_id('x', id)[2..]);
    match (scope.root, scope.thread) {
        (None, None) => "all".to_owned(),
        (Some(root), None) => encode("root", root),
        (None, Some(thread)) => encode("thread", thread),
        (Some(root), Some(thread)) => {
            format!("{}-{}", encode("root", root), encode("thread", thread))
        },
    }
}

/// What a node or cluster shows: its id and title, with a conflicted title's values joined.
fn label(id: &str, title: &Value<String>) -> String {
    let title = title.all().into_iter().map(String::as_str).collect::<Vec<_>>().join(" | ");
    let text = if title.is_empty() { id.to_owned() } else { format!("{id} · {title}") };
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn item_label(model: &Model, id: &str) -> String {
    model.items.get(id).map_or_else(|| id.to_owned(), |item| label(id, &item.title))
}

fn thread_label(model: &Model, id: &str) -> String {
    model.threads.get(id).map_or_else(|| id.to_owned(), |thread| label(id, &thread.title))
}

fn is_done(model: &Model, id: &str) -> bool {
    model.items.get(id).is_some_and(crate::model::Item::is_done)
}

/// A DOT string literal. Graphviz reads HTML entities in quoted labels, so `&` is one too.
fn dot_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"").replace('&', "&amp;"))
}

/// The view as a Graphviz DOT document.
#[must_use]
pub fn dot(model: &Model, view: &View) -> String {
    let mut out = String::from("digraph acyclic {\n");
    out.push_str("  fontname=\"Helvetica,Arial,sans-serif\";\n");
    out.push_str("  node [shape=box, style=rounded, fontname=\"Helvetica,Arial,sans-serif\"];\n");
    let node = |out: &mut String, indent: &str, id: &str| {
        let style = if is_done(model, id) {
            ", style=\"rounded,filled\", fillcolor=\"#e8e8e8\""
        } else {
            ""
        };
        writeln!(
            out,
            "{indent}{} [label={}{style}];",
            dot_string(&node_id('n', id)),
            dot_string(&item_label(model, id))
        )
        .expect("a String takes writes");
    };
    let clustered: BTreeSet<&String> = view.clusters.values().flatten().collect();
    for (thread, members) in &view.clusters {
        writeln!(
            out,
            "  subgraph {} {{\n    label={};",
            dot_string(&format!("cluster_{}", node_id('t', thread))),
            dot_string(&thread_label(model, thread))
        )
        .expect("a String takes writes");
        for id in members {
            node(&mut out, "    ", id);
        }
        out.push_str("  }\n");
    }
    for id in view.items.iter().filter(|id| !clustered.contains(id)) {
        node(&mut out, "  ", id);
    }
    for id in &view.ghosts {
        writeln!(
            out,
            "  {} [label={}, style=\"rounded,dashed\", color=gray, fontcolor=gray];",
            dot_string(&node_id('n', id)),
            dot_string(&item_label(model, id))
        )
        .expect("a String takes writes");
    }
    for edge in &view.edges {
        let style = match edge.kind {
            EdgeKind::Tree => "color=gray, arrowhead=none",
            EdgeKind::Link(LinkKind::Blocks) => "style=solid",
            // The model reads a stored after link as relates.
            EdgeKind::Link(LinkKind::After | LinkKind::Relates) => "style=dotted, arrowhead=none",
        };
        writeln!(
            out,
            "  {} -> {} [{style}];",
            dot_string(&node_id('n', &edge.from)),
            dot_string(&node_id('n', &edge.to))
        )
        .expect("a String takes writes");
    }
    out.push_str("}\n");
    out
}

/// A Mermaid quoted label, with the characters Mermaid reads as syntax as its entity codes.
fn mermaid_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("#quot;"),
            '#' => out.push_str("#35;"),
            '<' => out.push_str("#lt;"),
            '>' => out.push_str("#gt;"),
            '&' => out.push_str("#amp;"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The view as a Mermaid flowchart.
#[must_use]
pub fn mermaid(model: &Model, view: &View) -> String {
    let mut out = String::from("flowchart TB\n");
    let node = |out: &mut String, indent: &str, id: &str| {
        let class = if is_done(model, id) { ":::done" } else { "" };
        writeln!(
            out,
            "{indent}{}[{}]{class}",
            node_id('n', id),
            mermaid_string(&item_label(model, id))
        )
        .expect("a String takes writes");
    };
    let clustered: BTreeSet<&String> = view.clusters.values().flatten().collect();
    for (thread, members) in &view.clusters {
        writeln!(
            out,
            "  subgraph {}[{}]",
            node_id('t', thread),
            mermaid_string(&thread_label(model, thread))
        )
        .expect("a String takes writes");
        for id in members {
            node(&mut out, "    ", id);
        }
        out.push_str("  end\n");
    }
    for id in view.items.iter().filter(|id| !clustered.contains(id)) {
        node(&mut out, "  ", id);
    }
    for id in &view.ghosts {
        writeln!(out, "  {}[{}]:::ghost", node_id('n', id), mermaid_string(&item_label(model, id)))
            .expect("a String takes writes");
    }
    let mut tree = Vec::new();
    for (index, edge) in view.edges.iter().enumerate() {
        let arrow = match edge.kind {
            EdgeKind::Tree => {
                tree.push(index.to_string());
                "---"
            },
            EdgeKind::Link(LinkKind::Blocks) => "-->",
            EdgeKind::Link(LinkKind::After | LinkKind::Relates) => "-.-",
        };
        writeln!(out, "  {} {arrow} {}", node_id('n', &edge.from), node_id('n', &edge.to))
            .expect("a String takes writes");
    }
    if !tree.is_empty() {
        writeln!(out, "  linkStyle {} stroke:#bbbbbb", tree.join(","))
            .expect("a String takes writes");
    }
    out.push_str("  classDef done fill:#e8e8e8\n");
    out.push_str("  classDef ghost stroke-dasharray:5 5,color:#888888\n");
    out
}

#[cfg(test)]
mod tests;
