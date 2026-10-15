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
#[must_use]
pub fn live(state: &State, item: &Key, now: Millis) -> Vec<Claim> {
    let mut claims: Vec<Claim> = state
        .field(item, FIELD)
        .into_iter()
        .flatten()
        .filter_map(|(_, value)| serde_json::from_value::<Claim>(value.value.clone()).ok())
        .filter(|claim| claim.is_live(now))
        .collect();
    claims.sort_by(|a, b| (a.lease_start, &a.replica).cmp(&(b.lease_start, &b.replica)));
    claims
}

/// The claim that wins at `now`: the same on every replica holding the same state.
#[must_use]
pub fn winner(state: &State, item: &Key, now: Millis) -> Option<Claim> {
    live(state, item, now).into_iter().next()
}

/// Claims an item for `holder`, or renews its own claim. A time lease runs for `lease`, capped at `max_lease`.
///
/// # Panics
///
/// Never: a claim always serializes and a write with a value always mints a dot.
///
/// # Errors
///
/// Fails when another holder's claim is live in this replica's view, so a claim never silently replaces one it has
/// seen; only claims made without seeing each other contest.
pub fn claim(
    replica: &mut Replica,
    item: &Key,
    holder: &str,
    at: Millis,
    lease: Option<Millis>,
    max_lease: Millis,
) -> Result<Dot, Error> {
    if live(&replica.state, item, at).iter().any(|claim| claim.holder != holder) {
        return Err(Error::Claimed(item.clone()));
    }
    let claim = Claim {
        holder: holder.to_owned(),
        replica: replica.id().as_str().to_owned(),
        lease_start: at,
        lease_expires: lease.map(|lease| at.saturating_add(lease.clamp(0, max_lease))),
    };
    let value = serde_json::to_value(claim).expect("a claim always serializes");
    let dot = replica.write(item, FIELD, holder, at, Some(value))?;
    Ok(dot.expect("a write with a value mints a dot"))
}
