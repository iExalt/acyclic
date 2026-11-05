//! Claims on threads, kept as a field so concurrent claims survive a join as a contest that every replica resolves
//! the same way. A thread is the unit of work: its holder works every member without claiming items. Claims older
//! versions kept on items are never read.

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::model::Model;
use crate::ready::{Ancestry, ReadinessEvaluator, ThreadReadiness};
use crate::schema::kind;
use crate::state::{CLOCK_SKEW_MS, Dot, Error, Key, Millis, Replica, State};

/// The field holding a thread's claims.
pub const FIELD: &str = "claim";

/// One claim on a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    /// Who holds it, such as an agent conversation.
    pub holder: String,
    /// The replica that recorded it.
    pub replica: String,
    /// When the lease began.
    pub lease_start: Millis,
    /// When the lease ends. Older versions wrote `None` for a claim that lived while its run did; such a claim no
    /// longer holds.
    pub lease_expires: Option<Millis>,
}

impl Claim {
    /// Whether the lease starts in the reader's future beyond the clock skew: the writer's clock ran ahead, an
    /// anomaly for the needs-you list.
    #[must_use]
    pub fn starts_in_future(&self, now: Millis) -> bool {
        self.lease_start > now.saturating_add(CLOCK_SKEW_MS)
    }
}

/// Every claim on an entity that holds at `now`, in the order that decides a contest: earliest lease, then replica.
/// A claim without a time lease, which only older versions wrote, never holds.
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
        if claim.lease_expires.is_some_and(|expires| now < expires) {
            claims.push(claim);
        }
    }
    claims.sort_by(|a, b| (a.lease_start, &a.replica).cmp(&(b.lease_start, &b.replica)));
    Ok(claims)
}

/// The claim that wins at `now`: the same on every replica holding the same state. A contest's loser must check this
/// rather than assume it holds the thread.
///
/// # Errors
///
/// Fails when a claim value can't be read.
pub fn winner(state: &State, item: &Key, now: Millis) -> Result<Option<Claim>, Error> {
    Ok(live(state, item, now)?.into_iter().next())
}

/// The distinct holders of `claims`, in their order: one holder's several claims are one claim on the thread.
#[must_use]
pub fn holders(claims: &[Claim]) -> Vec<String> {
    let mut holders: Vec<String> = Vec::new();
    for claim in claims {
        if !holders.contains(&claim.holder) {
            holders.push(claim.holder.clone());
        }
    }
    holders
}

/// Releases `holder`'s claims on an entity and no one else's, so a contest's loser can step aside without taking the
/// winner's claim with it, as any write to the field would. Returns whether it held one.
pub fn release(replica: &mut Replica, item: &Key, holder: &str) -> bool {
    replica.retain(item, FIELD, |stamped| {
        serde_json::from_value::<Claim>(stamped.value.clone())
            .map_or(true, |claim| claim.holder != holder)
    })
}

/// The key a thread's claims are kept on.
///
/// # Errors
///
/// Fails on an invalid thread id.
pub fn thread_key(thread: &str) -> Result<Key, Error> {
    Key::new(kind::THREAD, thread)
}

/// The live claims that cover an item: those on its thread, when it is in exactly one thread that has a record.
///
/// # Errors
///
/// Fails when a claim can't be read.
pub fn covering(
    state: &State,
    model: &Model,
    item: &str,
    now: Millis,
) -> Result<Vec<Claim>, Error> {
    match model.thread_of(item).filter(|thread| model.threads.contains_key(*thread)) {
        Some(thread) => live(state, &thread_key(thread)?, now),
        None => Ok(Vec::new()),
    }
}

/// Claims a thread for `holder`, or renews the claim `holder` already wins. The lease runs for `lease` from `at`,
/// capped at `max_lease`. A renewal keeps its original lease start, so renewing never loses a contest, and its write
/// supersedes the other contestants' values: an ordinary resolving write.
///
/// # Panics
///
/// Never: a claim always serializes and a write with a value always mints a dot.
///
/// Unless forced, which `ready` of `None` is, the thread must be claimable by `holder`: ready, with merges checked
/// through `ready`, or won by `holder` already.
///
/// # Errors
///
/// Fails when the thread has no record, when it isn't claimable and the claim isn't forced, when another
/// holder wins the thread in this replica's view (so a claim never silently replaces one it has seen; only claims made
/// without seeing each other contest), or on an unreadable claim.
pub fn claim(
    replica: &mut Replica,
    thread: &str,
    holder: &str,
    at: Millis,
    (lease, max_lease): (Millis, Millis),
    ready: Option<&dyn Ancestry>,
) -> Result<Dot, ClaimError> {
    let model = Model::read(&replica.state)?;
    if !model.threads.contains_key(thread) {
        return Err(Error::NoThread(thread.to_owned()).into());
    }
    if let Some(ancestry) = ready {
        let status =
            ReadinessEvaluator::new(&replica.state, &model, at, ancestry).thread(thread)?;
        if !status.claimable_by(holder) {
            return Err(ClaimError::NotReady(status));
        }
    }
    let key = thread_key(thread)?;
    let current = winner(&replica.state, &key, at)?;
    if current.as_ref().is_some_and(|claim| claim.holder != holder) {
        return Err(Error::Claimed(key).into());
    }
    let claim = Claim {
        holder: holder.to_owned(),
        replica: replica.id().as_str().to_owned(),
        lease_start: current.map_or(at, |claim| claim.lease_start),
        lease_expires: Some(at.saturating_add(lease.clamp(0, max_lease))),
    };
    let value = serde_json::to_value(claim).expect("a claim always serializes");
    let dot = replica.write(&key, FIELD, holder, at, Some(value))?;
    Ok(dot.expect("a write with a value mints a dot"))
}

/// Why a [`claim`] wasn't made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimError {
    /// The thread isn't the holder's to claim without forcing: it has no ready work, or someone else holds it.
    NotReady(ThreadReadiness),
    /// The thread has no record, someone else wins it, or a claim can't be read or written.
    State(Error),
}

impl From<Error> for ClaimError {
    fn from(err: Error) -> Self {
        Self::State(err)
    }
}

/// Renews the time leases this replica recorded whose holders are still at work: each winning thread claim with less
/// than half of `lease` left whose holder wrote something other than a claim within the last `lease`. The CLI calls it
/// at the start of each of its write batches, whoever writes; a sync doesn't. A holder that stopped writing, such as a
/// conversation that ended, keeps no claim past its lease. Renewals keep their lease start and are written by the replica.
///
/// # Panics
///
/// Never: a claim always serializes.
///
/// # Errors
///
/// Fails on an unreadable claim or when the replica can't write.
pub fn renew_leases(
    replica: &mut Replica,
    now: Millis,
    lease: Millis,
    max_lease: Millis,
) -> Result<Vec<Key>, Error> {
    let mut last_active: BTreeMap<&str, Millis> = BTreeMap::new();
    for (_, entity) in replica.state.entities() {
        for (_, stamped) in entity.iter().filter(|(name, _)| *name != FIELD).flat_map(|(_, r)| r) {
            let last = last_active.entry(stamped.actor.as_str()).or_insert(stamped.at);
            *last = (*last).max(stamped.at);
        }
    }
    let mine = replica.id().as_str().to_owned();
    let mut renewals = Vec::new();
    for (key, entity) in replica.state.entities() {
        // Claims older versions kept on items are never read, so never renewed.
        if key.kind != kind::THREAD || !entity.contains_key(FIELD) {
            continue;
        }
        let Some(claim) = winner(&replica.state, key, now)? else { continue };
        let active = last_active
            .get(claim.holder.as_str())
            .is_some_and(|&at| at >= now.saturating_sub(lease));
        // Only a lease past half its term is renewed, so a burst of writes renews each claim once.
        let due =
            claim.lease_expires.is_some_and(|expires| expires.saturating_sub(now) < lease / 2);
        if claim.replica == mine && due && active {
            renewals.push((key.clone(), claim));
        }
    }
    let actor = format!("replica/{mine}");
    for (key, claim) in &renewals {
        let renewed = Claim {
            lease_expires: Some(now.saturating_add(lease.clamp(0, max_lease))),
            ..claim.clone()
        };
        let value = serde_json::to_value(renewed).expect("a claim always serializes");
        replica.write(key, FIELD, &actor, now, Some(value))?;
    }
    Ok(renewals.into_iter().map(|(key, _)| key).collect())
}

#[cfg(test)]
mod tests;
