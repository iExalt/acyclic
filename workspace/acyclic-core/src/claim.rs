//! Claims on threads, kept as a field so concurrent claims survive a join as a contest that every replica resolves
//! the same way. A thread is the unit of work: its holder works every member without claiming items, and its claim
//! holds the named resources of every open member. Claims older versions kept on items are never read.

use serde::{Deserialize, Serialize};

use std::collections::{BTreeMap, BTreeSet};

use crate::model::Model;
use crate::schema::kind;
use crate::state::{CLOCK_SKEW_MS, Dot, Error, Key, Millis, Replica, State};

/// The field holding a thread's claims.
pub const FIELD: &str = "claim";

/// One claim on a thread.
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

/// Whether a claim without a time lease is still held: the run behind it is alive.
pub type Alive<'a> = &'a dyn Fn(&str) -> bool;

/// Every claim on an entity that holds at `now`, in the order that decides a contest: earliest lease, then replica.
/// Claims without a time lease all count as held.
///
/// # Errors
///
/// Fails when a claim value can't be read, such as one from a newer version: hiding it would let anyone claim the
/// item.
pub fn live(state: &State, item: &Key, now: Millis) -> Result<Vec<Claim>, Error> {
    live_with(state, item, now, &|_| true)
}

/// Like [`live`], where a claim without a time lease holds only while `alive` says its holder is.
///
/// # Errors
///
/// Fails when a claim value can't be read.
pub fn live_with(
    state: &State,
    item: &Key,
    now: Millis,
    alive: Alive<'_>,
) -> Result<Vec<Claim>, Error> {
    let mut claims = Vec::new();
    for value in state.field(item, FIELD).into_iter().flatten().map(|(_, v)| &v.value) {
        let claim: Claim = serde_json::from_value(value.clone()).map_err(|e| {
            Error::Format(format!("unreadable claim on {} {}: {e}", item.kind, item.id))
        })?;
        let held = match claim.lease_expires {
            Some(expires) => now < expires,
            None => alive(&claim.holder),
        };
        if held {
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

/// Like [`winner`], with run liveness from `alive`.
///
/// # Errors
///
/// Fails when a claim value can't be read.
pub fn winner_with(
    state: &State,
    item: &Key,
    now: Millis,
    alive: Alive<'_>,
) -> Result<Option<Claim>, Error> {
    Ok(live_with(state, item, now, alive)?.into_iter().next())
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
    alive: Alive<'_>,
) -> Result<Vec<Claim>, Error> {
    match model.thread_of(item).filter(|thread| model.threads.contains_key(*thread)) {
        Some(thread) => live_with(state, &thread_key(thread)?, now, alive),
        None => Ok(Vec::new()),
    }
}

/// The resources a claim on each thread holds, by thread: those its open members name, in one pass over the items.
/// Finished or archived work holds nothing, and a thread with no record or no such resource is left out.
#[must_use]
pub fn resources(model: &Model) -> BTreeMap<&str, BTreeSet<&str>> {
    let mut found: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for item in model.items.values().filter(|item| !item.resources.is_empty()) {
        let Some(thread) = item.thread.one().filter(|thread| model.threads.contains_key(*thread))
        else {
            continue;
        };
        if !item.is_done() && !model.is_archived(&item.id) {
            found.entry(thread).or_default().extend(item.resources.iter().map(String::as_str));
        }
    }
    found
}

/// A resource held elsewhere: the resource, the thread whose claim holds it, and that claim's holder.
pub type Held = (String, String, String);

/// A resource the thread's open members name that another thread's winning claim, held by someone other than
/// `holder`, holds: the resource, that thread and its holder. With no `holder`, any other thread's winner counts.
/// `held` is [`resources`], and `winner` gives a thread's winning claim.
///
/// # Errors
///
/// Fails when a claim can't be read.
pub fn resource_held(
    held: &BTreeMap<&str, BTreeSet<&str>>,
    thread: &str,
    holder: Option<&str>,
    winner: impl Fn(&str) -> Result<Option<Claim>, Error>,
) -> Result<Option<Held>, Error> {
    let Some(wanted) = held.get(thread) else { return Ok(None) };
    for (other, theirs) in held.iter().filter(|(other, _)| **other != thread) {
        let Some(resource) = theirs.intersection(wanted).next() else { continue };
        if let Some(claim) = winner(other)?
            && holder != Some(claim.holder.as_str())
        {
            return Ok(Some(((*resource).to_owned(), (*other).to_owned(), claim.holder)));
        }
    }
    Ok(None)
}

/// Claims a thread for `holder`, or renews the claim `holder` already wins. A time lease runs for `lease` from `at`,
/// capped at `max_lease`. A renewal keeps its original lease start, so renewing never loses a contest, and its write
/// supersedes the other contestants' values: an ordinary resolving write. Run liveness comes from the run records
/// (see [`Model::run_alive`]).
///
/// # Panics
///
/// Never: a claim always serializes and a write with a value always mints a dot.
///
/// # Errors
///
/// Fails when a holder other than a run claims without a time lease, when the thread has no record, when another
/// holder wins the thread in this replica's view (so a claim never silently replaces one it has seen; only claims made
/// without seeing each other contest), when another holder's winning claim on another thread holds a resource one of
/// this thread's open members names, or on an unreadable claim.
pub fn claim(
    replica: &mut Replica,
    thread: &str,
    holder: &str,
    at: Millis,
    lease: Option<Millis>,
    max_lease: Millis,
) -> Result<Dot, Error> {
    // Only a run has a liveness signal; any other holder must take a time lease, or its claim would never end.
    if lease.is_none() && !holder.starts_with("run/") {
        return Err(Error::LeaseRequired(holder.to_owned()));
    }
    let model = Model::read(&replica.state)?;
    if !model.threads.contains_key(thread) {
        return Err(Error::NoThread(thread.to_owned()));
    }
    let key = thread_key(thread)?;
    let alive = |holder: &str| model.run_alive(holder);
    let current = winner_with(&replica.state, &key, at, &alive)?;
    if current.as_ref().is_some_and(|claim| claim.holder != holder) {
        return Err(Error::Claimed(key));
    }
    let winner = |other: &str| winner_with(&replica.state, &thread_key(other)?, at, &alive);
    if let Some((resource, thread, holder)) =
        resource_held(&resources(&model), thread, Some(holder), winner)?
    {
        return Err(Error::ResourceHeld { resource, thread, holder });
    }
    let claim = Claim {
        holder: holder.to_owned(),
        replica: replica.id().as_str().to_owned(),
        lease_start: current.map_or(at, |claim| claim.lease_start),
        lease_expires: lease.map(|lease| at.saturating_add(lease.clamp(0, max_lease))),
    };
    let value = serde_json::to_value(claim).expect("a claim always serializes");
    let dot = replica.write(&key, FIELD, holder, at, Some(value))?;
    Ok(dot.expect("a write with a value mints a dot"))
}

/// Renews the time leases this replica recorded whose holders are still at work: each winning thread claim with less
/// than half of `lease` left whose holder wrote something other than a claim within the last `lease`. The CLI calls it
/// at the start of each of its write batches, whoever writes; a sync doesn't. A holder that stopped writing, such as a
/// session that ended, keeps no claim past its lease. Renewals keep their lease start and are written by the replica.
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

/// Threads holding a named resource through open members whose winning claims belong to different holders: a clash
/// only a join makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clash {
    /// The resource.
    pub resource: String,
    /// Each claimed thread holding it, with its winning holder.
    pub holders: Vec<(String, String)>,
}

/// The resource clashes at `now`.
///
/// # Errors
///
/// Fails on an unreadable claim.
pub fn clashes(state: &State, model: &Model, now: Millis) -> Result<Vec<Clash>, Error> {
    let alive = |holder: &str| model.run_alive(holder);
    let mut by_resource: BTreeMap<&str, Vec<(String, String)>> = BTreeMap::new();
    for (thread, held) in resources(model) {
        if let Some(claim) = winner_with(state, &thread_key(thread)?, now, &alive)? {
            for resource in held {
                by_resource
                    .entry(resource)
                    .or_default()
                    .push((thread.to_owned(), claim.holder.clone()));
            }
        }
    }
    Ok(by_resource
        .into_iter()
        .filter(|(_, holders)| holders.iter().any(|(_, h)| *h != holders[0].1))
        .map(|(resource, holders)| Clash { resource: resource.to_owned(), holders })
        .collect())
}

#[cfg(test)]
mod tests;
