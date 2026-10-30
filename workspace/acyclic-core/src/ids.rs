//! Ids the tracker assigns, so that agents stop guessing the next number.
//!
//! An assigned id is `<kind>-<replica>-<n>`, such as `item-mac-143` or `thread-mac-32`. `n` is one more than the
//! highest number that ends an id of that kind starting with `<kind>-`: assigned ids, and older ones such as
//! `thread-27`, deleted ones included, so no number is ever handed out twice on one replica. Other ids, such as
//! `side-5`, don't count, so a year in a name can't push the count up. An imported or older id such as `item-2026`
//! still does; new explicit ids starting with `<kind>-` are refused, so only those can.
//!
//! The replica's own id keeps two replicas' assignments apart, so concurrent adds on two clones never collide, though
//! they can end in the same number.
//!
//! Assign inside the write that adds the record: a write starts from the tracker's latest state under its lock, so
//! two adds on one replica can't both see the same highest number.

use crate::schema::{item, kind};
use crate::state::{ReplicaId, State};

/// The number that ends an id of `kind` in the tracker's own form, `<kind>-<n>` or `<kind>-<replica>-<n>`, if it is
/// one.
#[must_use]
pub fn number(kind: &str, id: &str) -> Option<u64> {
    let rest = id.strip_prefix(kind)?.strip_prefix('-')?;
    if !rest.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return None;
    }
    let digits = rest.rsplit('-').next()?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Whether a new explicit id of `kind` is the tracker's to give: it starts with `<kind>-`.
#[must_use]
pub fn reserved(kind: &str, id: &str) -> bool {
    id.strip_prefix(kind).is_some_and(|rest| rest.starts_with('-'))
}

/// The next id of `kind` this replica assigns. Threads count every id the state names, including those only an
/// item's membership names.
#[must_use]
pub fn next(state: &State, kind: &str, replica: &ReplicaId) -> String {
    let named = state.entities().flat_map(|(key, entity)| {
        let own = (key.kind == kind).then_some(key.id.as_str());
        let member = (kind == kind::THREAD && key.kind == kind::ITEM)
            .then(|| entity.get(item::THREAD))
            .flatten()
            .into_iter()
            .flat_map(|register| register.values().filter_map(|stamped| stamped.value.as_str()));
        own.into_iter().chain(member)
    });
    let highest = named.filter_map(|id| number(kind, id)).max().unwrap_or(0);
    format!("{kind}-{}-{}", replica.as_str(), highest.saturating_add(1))
}

#[cfg(test)]
mod tests;
