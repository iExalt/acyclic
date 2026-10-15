//! A local cache of this replica's parsed state, so a read needn't list and parse every file in the state's tree.
//!
//! Two files per replica under the common Git directory. The base holds a header line naming the build and the
//! state ref's tip it holds, then the whole state as a compact snapshot. The delta holds a header line naming the
//! build, the base's tip and its own tip, then the context and every entity changed since the base, with tombstones
//! for removed ones. A batch rewrites only the delta, so a write costs what changed since the base rather than the
//! whole state, until the delta outgrows a sixteenth of the base and the batch writes a new base instead.
//!
//! A reader trusts the base alone for the base's tip, and the base with the delta only for the delta's tip when the
//! delta names that base; anything else (another tip, another build, a damaged file) is a miss that falls back to
//! Git. Writes are atomic renames, so a reader sees a whole file or none, and a base and delta written by different
//! processes never combine. Anything able to write here can already move the state ref.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::state::{Key, ReplicaId, State};

/// A delta larger than this share of the base's size is folded into a new base.
const DELTA_SHARE: u64 = 16;

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Header {
    acyclic: String,
    tip: String,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct DeltaHeader {
    acyclic: String,
    base: String,
    tip: String,
}

/// The cache files for one replica.
#[derive(Debug, Clone)]
pub(crate) struct Cache {
    base: PathBuf,
    delta: PathBuf,
}

/// What a read found, for the next write to extend: the base it used, with its size, and the keys its delta held.
#[derive(Debug, Clone, Default)]
pub(crate) struct Found {
    base: Option<(String, u64)>,
    delta: BTreeSet<Key>,
}

impl Cache {
    pub(crate) fn new(common_dir: &Path, replica: &ReplicaId) -> Self {
        let dir = common_dir.join("acyclic").join("cache");
        Self {
            base: dir.join(format!("{replica}.json")),
            delta: dir.join(format!("{replica}.delta.json")),
        }
    }

    /// The state cached for `tip`, if this build wrote it, and what held it.
    pub(crate) fn read(&self, tip: &str) -> Option<(State, Found)> {
        let base = std::fs::read_to_string(&self.base).ok();
        crate::probe::mark("cache: read file");
        let Some((base_tip, snapshot)) = base.as_deref().and_then(|text| {
            let (header, snapshot) = text.split_once('\n')?;
            let header: Header = serde_json::from_str(header).ok()?;
            (header.acyclic == crate::VERSION).then_some((header.tip, snapshot))
        }) else {
            crate::probe::mark("cache: miss");
            return None;
        };
        let delta = if base_tip == tip {
            None
        } else {
            let text = std::fs::read_to_string(&self.delta).ok();
            crate::probe::mark("cache: read delta");
            let expected = DeltaHeader {
                acyclic: crate::VERSION.to_owned(),
                base: base_tip.clone(),
                tip: tip.to_owned(),
            };
            let delta = text.and_then(|text| {
                let (header, delta) = text.split_once('\n')?;
                let header: DeltaHeader = serde_json::from_str(header).ok()?;
                (header == expected).then(|| delta.to_owned())
            });
            if delta.is_none() {
                crate::probe::mark("cache: miss");
                return None;
            }
            delta
        };
        let mut state = State::from_snapshot(snapshot).ok()?;
        crate::probe::mark("cache: parse");
        let keys = match delta {
            Some(delta) => state.apply_delta(&delta).ok()?,
            None => BTreeSet::new(),
        };
        crate::probe::mark("cache: apply delta");
        let size = base.map_or(0, |text| text.len() as u64);
        Some((state, Found { base: Some((base_tip, size)), delta: keys }))
    }

    /// Caches `state` as the one at `tip`, after a batch changed the entities at `changed` in the state a read
    /// `found`: as a delta on that read's base while the delta stays small, and as a new base otherwise. Best effort:
    /// a failure only means the next read goes to Git.
    pub(crate) fn write_after<'a>(
        &self,
        found: &'a Found,
        tip: &str,
        state: &State,
        changed: impl Iterator<Item = &'a Key>,
    ) {
        let Some((base, size)) = &found.base else {
            self.write(tip, state);
            return;
        };
        let keys: BTreeSet<&Key> = found.delta.iter().chain(changed).collect();
        let header = DeltaHeader {
            acyclic: crate::VERSION.to_owned(),
            base: base.clone(),
            tip: tip.to_owned(),
        };
        let header = serde_json::to_string(&header).expect("a header serializes");
        let text = format!("{header}\n{}", state.to_delta(keys.into_iter()));
        crate::probe::mark("cache: serialize delta");
        if text.len() as u64 > size / DELTA_SHARE {
            self.write(tip, state);
            return;
        }
        replace(&self.delta, text.as_bytes());
        crate::probe::mark("cache: write delta");
    }

    /// Caches `state` as the one at `tip`, as a new base, and returns it as a read would find it. Best effort.
    pub(crate) fn write(&self, tip: &str, state: &State) -> Found {
        let header = Header { acyclic: crate::VERSION.to_owned(), tip: tip.to_owned() };
        let header = serde_json::to_string(&header).expect("a header serializes");
        let text = format!("{header}\n{}", state.to_snapshot());
        crate::probe::mark("cache: serialize");
        let written = replace(&self.base, text.as_bytes());
        crate::probe::mark("cache: write file");
        let base = written.then(|| (tip.to_owned(), text.len() as u64));
        Found { base, delta: BTreeSet::new() }
    }

    #[cfg(test)]
    pub(crate) fn base(&self) -> &Path {
        &self.base
    }
}

/// Replaces a file atomically, through a temporary file and a rename, reporting whether it did. Best effort.
fn replace(path: &Path, content: &[u8]) -> bool {
    let Some(dir) = path.parent() else { return false };
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let temporary = dir.join(format!(
        ".{}.{}.{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("cache"),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    if std::fs::write(&temporary, content).is_err() || std::fs::rename(&temporary, path).is_err() {
        let _ = std::fs::remove_file(&temporary);
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::state::Replica;

    fn cache(dir: &Path) -> Cache {
        Cache::new(dir, &ReplicaId::new("mac").unwrap())
    }

    fn key(id: &str) -> Key {
        Key::new("item", id).unwrap()
    }

    /// A state of `items` items with long titles, so the base dwarfs a one-item delta.
    fn state(items: usize) -> State {
        let mut replica = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
        for n in 0..items {
            let title = format!("Item {n} {}", "x".repeat(40));
            replica
                .write(&key(&format!("a{n}")), "title", "actor/user", 1, Some(json!(title)))
                .unwrap();
        }
        replica.into_state()
    }

    fn write(state: State, id: &str, value: Option<&str>) -> State {
        let mut replica = Replica::open(ReplicaId::new("mac").unwrap(), state);
        replica.write(&key(id), "title", "actor/user", 2, value.map(|v| json!(v))).unwrap();
        replica.into_state()
    }

    #[test]
    fn a_cache_serves_only_its_own_tip_and_build() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = cache(dir.path());
        assert!(cache.read("t1").is_none(), "nothing cached yet");
        cache.write("t1", &state(1));
        assert_eq!(cache.read("t1").map(|(s, _)| s), Some(state(1)));
        assert!(cache.read("t2").is_none(), "another tip");
        let text = std::fs::read_to_string(cache.base()).unwrap();
        std::fs::write(cache.base(), text.replacen(crate::VERSION, "0.0.0-other", 1)).unwrap();
        assert!(cache.read("t1").is_none(), "another build");
        std::fs::write(cache.base(), &text[..text.len() / 2]).unwrap();
        assert!(cache.read("t1").is_none(), "a damaged file");
    }

    #[test]
    fn deltas_accumulate_on_their_base_until_they_outgrow_it() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = cache(dir.path());
        let t1 = state(100);
        cache.write("t1", &t1);
        let base = std::fs::read_to_string(cache.base()).unwrap();

        // An edit, then a removal: each a delta on the same base, which is never rewritten.
        let (_, found) = cache.read("t1").unwrap();
        let t2 = write(t1.clone(), "a1", Some("changed"));
        cache.write_after(&found, "t2", &t2, [key("a1")].iter());
        let (read, found) = cache.read("t2").unwrap();
        assert_eq!(read, t2);
        let t3 = write(t2, "a2", None);
        cache.write_after(&found, "t3", &t3, [key("a2")].iter());
        assert_eq!(
            cache.read("t3").map(|(s, _)| s),
            Some(t3.clone()),
            "the removal is a tombstone"
        );
        assert_eq!(std::fs::read_to_string(cache.base()).unwrap(), base);
        assert!(cache.read("t2").is_none(), "the delta serves only its own tip");

        // Another process's base: the delta no longer names it.
        cache.write("t9", &t1);
        assert!(cache.read("t3").is_none());

        // Changes past a sixteenth of the base fold into a new base.
        cache.write("t3", &t3);
        let (_, found) = cache.read("t3").unwrap();
        let changed: Vec<Key> = (0..10).map(|n| key(&format!("a{n}"))).collect();
        let mut t4 = t3;
        for key in &changed {
            t4 = write(t4, &key.id, Some("changed again"));
        }
        cache.write_after(&found, "t4", &t4, changed.iter());
        assert_eq!(cache.read("t4").map(|(s, _)| s), Some(t4));
        let base = std::fs::read_to_string(cache.base()).unwrap();
        assert!(
            base.starts_with(&format!("{{\"acyclic\":\"{}\",\"tip\":\"t4\"}}", crate::VERSION))
        );
    }
}
