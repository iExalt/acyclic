//! Claims on items, kept as a field so concurrent claims survive a join as a contest that every replica resolves the
//! same way.

use serde::{Deserialize, Serialize};

use crate::state::{CLOCK_SKEW_MS, Dot, Error, Key, Millis, Replica, State};

/// The field holding an item's claims.
pub const FIELD: &str = "claim";

/// One claim on an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    /// Who holds it: a run or an interactive session.
    pub holder: String,
    /// The replica that recorded it.
    pub replica: String,
    /// When the lease began.
    pub lease_start: Millis,
    /// When a time lease ends; `None` for a claim that lives while its run is alive.
    pub lease_expires: Option<Millis>,
}

impl Claim {
    /// Whether the claim holds at `now`.
    #[must_use]
    pub fn is_live(&self, now: Millis) -> bool {
        self.lease_expires.is_none_or(|expires| now < expires)
    }

    /// Whether the lease starts in the reader's future beyond the clock skew: the writer's clock ran ahead, an
    /// anomaly for the needs-you list.
    #[must_use]
    pub fn starts_in_future(&self, now: Millis) -> bool {
        self.lease_start > now.saturating_add(CLOCK_SKEW_MS)
    }
}

/// Every claim on an item that holds at `now`, in the order that decides a contest: earliest lease, then replica.
///
/// # Errors
///
/// Fails when a claim value can't be read, such as one from a newer version: hiding it would let anyone claim the
/// item.
pub fn live(state: &State, item: &Key, now: Millis) -> Result<Vec<Claim>, Error> {
    let mut claims = Vec::new();
    for value in state.field(item, FIELD).into_iter().flatten().map(|(_, v)| &v.value) {
        let claim: Claim = serde_json::from_value(value.clone()).map_err(|e| {
            Error::Format(format!("unreadable claim on {} {}: {e}", item.kind, item.id))
        })?;
        if claim.is_live(now) {
            claims.push(claim);
        }
    }
    claims.sort_by(|a, b| (a.lease_start, &a.replica).cmp(&(b.lease_start, &b.replica)));
    Ok(claims)
}

/// The claim that wins at `now`: the same on every replica holding the same state. A contest's loser must check this
/// rather than assume it holds the item.
///
/// # Errors
///
/// Fails when a claim value can't be read.
pub fn winner(state: &State, item: &Key, now: Millis) -> Result<Option<Claim>, Error> {
    Ok(live(state, item, now)?.into_iter().next())
}

/// Claims an item for `holder`, or renews the claim `holder` already wins. A time lease runs for `lease` from `at`,
/// capped at `max_lease`. A renewal keeps its original lease start, so renewing never loses a contest, and its write
/// supersedes the other contestants' values: an ordinary resolving write.
///
/// # Panics
///
/// Never: a claim always serializes and a write with a value always mints a dot.
///
/// # Errors
///
/// Fails when another holder wins the item in this replica's view, so a claim never silently replaces one it has
/// seen; only claims made without seeing each other contest. Also fails on an unreadable claim.
pub fn claim(
    replica: &mut Replica,
    item: &Key,
    holder: &str,
    at: Millis,
    lease: Option<Millis>,
    max_lease: Millis,
) -> Result<Dot, Error> {
    let current = winner(&replica.state, item, at)?;
    if current.as_ref().is_some_and(|claim| claim.holder != holder) {
        return Err(Error::Claimed(item.clone()));
    }
    let claim = Claim {
        holder: holder.to_owned(),
        replica: replica.id().as_str().to_owned(),
        lease_start: current.map_or(at, |claim| claim.lease_start),
        lease_expires: lease.map(|lease| at.saturating_add(lease.clamp(0, max_lease))),
    };
    let value = serde_json::to_value(claim).expect("a claim always serializes");
    let dot = replica.write(item, FIELD, holder, at, Some(value))?;
    Ok(dot.expect("a write with a value mints a dot"))
}
