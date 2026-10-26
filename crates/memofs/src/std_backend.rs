use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::{collections::HashSet, io};

use crossbeam_channel::Receiver;
use notify::{watcher, DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{DirEntry, Metadata, ReadDir, VfsBackend, VfsEvent};

/// `VfsBackend` that uses `std::fs` and the `notify` crate.
pub struct StdBackend {
    watcher: RecommendedWatcher,
    watcher_receiver: Receiver<VfsEvent>,
    watches: HashSet<PathBuf>,
    parent_watches: HashSet<PathBuf>,
    directory_watches: Arc<Mutex<HashSet<PathBuf>>>,
}

impl StdBackend {
    pub fn new() -> io::Result<StdBackend> {
        let (notify_tx, notify_rx) = mpsc::channel();
        let watcher = watcher(notify_tx, Duration::from_millis(50)).map_err(io::Error::other)?;

        let (tx, rx) = crossbeam_channel::unbounded();
        let directory_watches = Arc::new(Mutex::new(HashSet::new()));
        let forwarding_directory_watches = Arc::clone(&directory_watches);

        thread::spawn(move || {
            for event in notify_rx {
                log::trace!("watcher event: {:?}", event);
                match event {
                    DebouncedEvent::Create(path) => {
                        tx.send(VfsEvent::Create(path))?;
                    }
                    DebouncedEvent::Write(path) => {
                        // Parent guards also report metadata writes on watched
                        // directories. Their recursive watch already reports the
                        // actual child changes; forwarding both duplicates work.
                        if path.is_dir()
                            && forwarding_directory_watches.lock().unwrap().contains(&path)
                        {
                            continue;
                        }
                        tx.send(VfsEvent::Write(path))?;
                    }
                    DebouncedEvent::Remove(path) => {
                        tx.send(VfsEvent::Remove(path))?;
                    }
                    DebouncedEvent::Rename(from, to) => {
                        tx.send(VfsEvent::Remove(from))?;
                        tx.send(VfsEvent::Create(to))?;
                    }
                    _ => {}
                }
            }

            Result::<(), crossbeam_channel::SendError<VfsEvent>>::Ok(())
        });

        Ok(Self {
            watcher,
            watcher_receiver: rx,
            watches: HashSet::new(),
            parent_watches: HashSet::new(),
            directory_watches,
        })
    }

    fn watch_parent(&mut self, path: &Path) -> io::Result<()> {
        let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        else {
            return Ok(());
        };
        if self.parent_watches.contains(parent)
            || parent
                .ancestors()
                .any(|ancestor| self.watches.contains(ancestor))
        {
            return Ok(());
        }
        // A watch on the source itself cannot see its own recreation. Only
        // watch the parent's immediate entries, rather than unrelated subtrees.
        log::trace!(
            "watch {}: registering parent non-recursively",
            parent.display()
        );
        self.watcher
            .watch(parent, RecursiveMode::NonRecursive)
            .map_err(io::Error::other)?;
        self.parent_watches.insert(parent.to_path_buf());
        Ok(())
    }
}

impl VfsBackend for StdBackend {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        fs_err::read(path)
    }

    fn write(&mut self, path: &Path, data: &[u8]) -> io::Result<()> {
        fs_err::write(path, data)
    }

    fn exists(&mut self, path: &Path) -> io::Result<bool> {
        std::fs::exists(path)
    }

    fn read_dir(&mut self, path: &Path) -> io::Result<ReadDir> {
        let entries: Result<Vec<_>, _> = fs_err::read_dir(path)?.collect();
        let mut entries = entries?;

        entries.sort_by_cached_key(|entry| entry.file_name());

        let inner = entries
            .into_iter()
            .map(|entry| Ok(DirEntry { path: entry.path() }));

        Ok(ReadDir {
            inner: Box::new(inner),
        })
    }

    fn create_dir(&mut self, path: &Path) -> io::Result<()> {
        fs_err::create_dir(path)
    }

    fn create_dir_all(&mut self, path: &Path) -> io::Result<()> {
        fs_err::create_dir_all(path)
    }

    fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        fs_err::remove_file(path)
    }

    fn remove_dir_all(&mut self, path: &Path) -> io::Result<()> {
        fs_err::remove_dir_all(path)
    }

    fn metadata(&mut self, path: &Path) -> io::Result<Metadata> {
        let inner = fs_err::metadata(path)?;

        Ok(Metadata {
            is_file: inner.is_file(),
        })
    }

    fn canonicalize(&mut self, path: &Path) -> io::Result<PathBuf> {
        dunce::canonicalize(path)
    }

    fn event_receiver(&self) -> crossbeam_channel::Receiver<VfsEvent> {
        self.watcher_receiver.clone()
    }

    fn watch(&mut self, path: &Path) -> io::Result<()> {
        if self.watches.contains(path) {
            return self.watch_parent(path);
        }
        if path
            .ancestors()
            .any(|ancestor| self.watches.contains(ancestor))
        {
            log::trace!("watch {}: covered by existing watch", path.display());
            Ok(())
        } else {
            self.watch_parent(path)?;
            if self.parent_watches.remove(path) {
                self.watcher.unwatch(path).map_err(io::Error::other)?;
            }
            log::trace!("watch {}: registering recursively", path.display());
            self.watcher
                .watch(path, RecursiveMode::Recursive)
                .map_err(io::Error::other)?;
            // Failed native registration must never count as an active watch.
            self.watches.insert(path.to_path_buf());
            if path.is_dir() {
                self.directory_watches
                    .lock()
                    .unwrap()
                    .insert(path.to_path_buf());
            }
            Ok(())
        }
    }

    fn unwatch(&mut self, path: &Path) -> io::Result<()> {
        log::trace!("unwatch {}", path.display());
        let recursive = self.watches.remove(path);
        self.directory_watches.lock().unwrap().remove(path);
        let parent = self.parent_watches.remove(path);
        if recursive || parent {
            self.watcher.unwatch(path).map_err(io::Error::other)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn failed_watch_registration_can_be_retried_after_creation() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let mut backend = StdBackend::new().unwrap();
        assert!(backend.watch(&missing).is_err());
        fs_err::create_dir(&missing).unwrap();
        backend.watch(&missing).unwrap();
        let created = missing.join("new.luau");
        fs_err::write(&created, "return 1").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let event = backend
                .event_receiver()
                .recv_deadline(deadline)
                .expect("retried watch never reported the new file");
            if matches!(event, VfsEvent::Create(path) if path == created) {
                break;
            }
        }
    }
}
