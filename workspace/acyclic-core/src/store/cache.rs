//! A local cache of this replica's parsed state, so a read needn't list and parse every file in the state's tree.
//!
//! One file per replica under the common Git directory: a header line naming the build and the state ref's tip it
//! holds, then the state as a compact snapshot. A reader trusts it only for that tip and that build, and anything
//! else (another tip, another build, a damaged file) is a miss that falls back to Git. Writes are atomic renames, so
//! a reader sees a whole file or none; anything able to write here can already move the state ref.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::state::{ReplicaId, State};

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Header {
    acyclic: String,
    tip: String,
}

/// The cache file for one replica.
#[derive(Debug, Clone)]
pub(crate) struct Cache {
    path: PathBuf,
}

impl Cache {
    pub(crate) fn new(common_dir: &Path, replica: &ReplicaId) -> Self {
        Self { path: common_dir.join("acyclic").join("cache").join(format!("{replica}.json")) }
    }

    fn header(tip: &str) -> Header {
        Header { acyclic: crate::VERSION.to_owned(), tip: tip.to_owned() }
    }

    /// The state cached for `tip`, if this build wrote it.
    pub(crate) fn read(&self, tip: &str) -> Option<State> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        let (header, snapshot) = text.split_once('\n')?;
        let header: Header = serde_json::from_str(header).ok()?;
        if header != Self::header(tip) {
            return None;
        }
        State::from_snapshot(snapshot).ok()
    }

    /// Caches `state` as the one at `tip`. Best effort: a failure only means the next read goes to Git.
    pub(crate) fn write(&self, tip: &str, state: &State) {
        let Some(dir) = self.path.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let header = serde_json::to_string(&Self::header(tip)).expect("a header serializes");
        let text = format!("{header}\n{}", state.to_snapshot());
        let temporary = dir.join(format!(
            ".{}.{}.{}",
            self.path.file_name().and_then(|n| n.to_str()).unwrap_or("cache"),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        if std::fs::write(&temporary, text).is_err()
            || std::fs::rename(&temporary, &self.path).is_err()
        {
            let _ = std::fs::remove_file(&temporary);
        }
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::state::{Key, Replica};

    fn cache(dir: &Path) -> Cache {
        Cache::new(dir, &ReplicaId::new("mac").unwrap())
    }

    fn state() -> State {
        let mut replica = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
        let key = Key::new("item", "a1").unwrap();
        replica.write(&key, "title", "actor/user", 1, Some(json!("First"))).unwrap();
        replica.state
    }

    #[test]
    fn a_cache_serves_only_its_own_tip_and_build() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = cache(dir.path());
        assert_eq!(cache.read("t1"), None, "nothing cached yet");
        cache.write("t1", &state());
        assert_eq!(cache.read("t1"), Some(state()));
        assert_eq!(cache.read("t2"), None, "another tip");
        let text = std::fs::read_to_string(cache.path()).unwrap();
        let other_build = text.replacen(crate::VERSION, "0.0.0-other", 1);
        std::fs::write(cache.path(), other_build).unwrap();
        assert_eq!(cache.read("t1"), None, "another build");
        std::fs::write(cache.path(), &text[..text.len() / 2]).unwrap();
        assert_eq!(cache.read("t1"), None, "a damaged file");
    }
}
