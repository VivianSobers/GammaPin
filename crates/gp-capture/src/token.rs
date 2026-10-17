//! Token hot-reload: the token is read again for every connection, and a file watcher
//! wakes a capture that is backing off as soon as a new token is written.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::Notify;

/// A fingerprint to notice that the token changed without keeping or logging it.
pub fn fingerprint(token: &str) -> u64 {
    let mut h = DefaultHasher::new();
    token.hash(&mut h);
    h.finish()
}

/// Watches the token file's directory (editors and `gp auth` replace files by rename).
pub struct TokenWatch {
    _watcher: RecommendedWatcher,
    changed: Arc<Notify>,
}

impl TokenWatch {
    pub fn new(token_file: &Path) -> notify::Result<Self> {
        let changed = Arc::new(Notify::new());
        let target: PathBuf = token_file.to_path_buf();
        let name = target.file_name().map(|n| n.to_os_string());
        let tx = changed.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(ev) = res else { return };
                let relevant = matches!(
                    ev.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                ) && ev
                    .paths
                    .iter()
                    .any(|p| p.file_name().map(|n| n.to_os_string()) == name);
                if relevant {
                    tx.notify_one();
                }
            })?;
        let dir = target
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        watcher.watch(dir, RecursiveMode::NonRecursive)?;
        Ok(Self {
            _watcher: watcher,
            changed,
        })
    }

    /// Notified on every change to the token file.
    pub fn changed(&self) -> Arc<Notify> {
        self.changed.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn notices_writes_and_renames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("upstox.token");
        let watch = TokenWatch::new(&path).unwrap();
        let changed = watch.changed();
        let wait = || tokio::time::timeout(Duration::from_secs(5), changed.notified());

        std::fs::write(&path, "first").unwrap();
        wait().await.expect("create noticed");
        let tmp = dir.path().join("upstox.token.new");
        std::fs::write(&tmp, "second").unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        wait().await.expect("rename noticed");

        // other files in the directory are ignored
        while tokio::time::timeout(Duration::from_millis(200), changed.notified())
            .await
            .is_ok()
        {}
        std::fs::write(dir.path().join("other"), "x").unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(300), changed.notified())
                .await
                .is_err()
        );
    }

    #[test]
    fn fingerprints_differ() {
        assert_ne!(fingerprint("a"), fingerprint("b"));
        assert_eq!(fingerprint("a"), fingerprint("a"));
    }
}
