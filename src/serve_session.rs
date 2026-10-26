use std::{
    collections::HashSet,
    io,
    net::IpAddr,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

use crossbeam_channel::Sender;
use memofs::Vfs;
use thiserror::Error;

use crate::{
    change_processor::ChangeProcessor,
    message_queue::MessageQueue,
    project::{Project, ProjectError},
    session_id::SessionId,
    snapshot::{
        apply_patch_set, compute_patch_set, AppliedPatchSet, InstanceContext, InstanceSnapshot,
        PatchSet, RojoTree,
    },
    snapshot_middleware::snapshot_from_vfs,
};

/// Contains all of the state for a Rojo serve session. A serve session is used
/// when we need to build a Rojo tree and possibly rebuild it when input files
/// change.
///
/// Nothing here is specific to any Rojo interface. Though the primary way to
/// interact with a serve session is Rojo's HTTP right now, there's no reason
/// why Rojo couldn't expose an IPC or channels-based API for embedding in the
/// future. `ServeSession` would be roughly the right interface to expose for
/// those cases.
pub struct ServeSession {
    /// The object responsible for listening to changes from the in-memory
    /// filesystem, applying them, updating the Roblox instance tree, and
    /// routing messages through the session's message queue to any connected
    /// clients.
    ///
    /// SHOULD BE DROPPED FIRST! ServeSession and ChangeProcessor communicate
    /// with eachother via channels. If ServeSession hangs up those channels
    /// before dropping the ChangeProcessor, its thread will panic with a
    /// RecvError, causing the main thread to panic on drop.
    ///
    /// Allowed to be unused because it has side effects when dropped.
    #[allow(unused)]
    change_processor: ChangeProcessor,

    /// When the serve session was started. Used only for user-facing
    /// diagnostics.
    start_time: Instant,

    /// The root project for the serve session.
    ///
    /// This will be defined if a folder with a `default.project.json` file was
    /// used for starting the serve session, or if the user specified a full
    /// path to a `.project.json` file.
    root_project: Project,

    /// A randomly generated ID for this serve session. It's used to ensure that
    /// a client doesn't begin connecting to a different server part way through
    /// an operation that needs to be atomic.
    session_id: SessionId,

    /// The tree of Roblox instances associated with this session that will be
    /// updated in real-time. This is derived from the session's VFS and will
    /// eventually be mutable to connected clients.
    tree: Arc<Mutex<RojoTree>>,

    /// An in-memory filesystem containing all of the files relevant for this
    /// live session.
    ///
    /// The main use for accessing it from the session is for debugging issues
    /// with Rojo's live-sync protocol.
    vfs: Arc<Vfs>,

    /// A queue of changes that have been applied to `tree` that affect clients.
    ///
    /// Clients to the serve session will subscribe to this queue either
    /// directly or through the HTTP API to be notified of mutations that need
    /// to be applied.
    message_queue: Arc<MessageQueue<AppliedPatchSet>>,

    /// A channel to send mutation requests on. These will be handled by the
    /// ChangeProcessor and trigger changes in the tree.
    tree_mutation_sender: Sender<PatchSet>,
    receipt_sender: Sender<crate::write_receipts::Command>,
    receipts: crate::write_receipts::Shared,
}

impl ServeSession {
    /// Start a new serve session from the given in-memory filesystem and start
    /// path.
    ///
    /// The project file is expected to be loaded out-of-band since it's
    /// currently loaded from the filesystem directly instead of through the
    /// in-memory filesystem layer.
    pub fn new<P: AsRef<Path>>(vfs: Vfs, start_path: P) -> Result<Self, ServeSessionError> {
        Self::new_with_context(vfs, start_path.as_ref(), false, false)
    }

    pub(crate) fn new_for_sourcemap<P: AsRef<Path>>(
        vfs: Vfs,
        start_path: P,
    ) -> Result<Self, ServeSessionError> {
        Self::new_with_context(vfs, start_path.as_ref(), true, true)
    }

    pub(crate) fn new_for_offline<P: AsRef<Path>>(
        vfs: Vfs,
        start_path: P,
    ) -> Result<Self, ServeSessionError> {
        Self::new_with_context(vfs, start_path.as_ref(), false, true)
    }

    fn new_with_context(
        vfs: Vfs,
        start_path: &Path,
        sourcemap_only: bool,
        offline_session: bool,
    ) -> Result<Self, ServeSessionError> {
        let _operation = crate::diagnostic_context::enter("loading serve session", start_path);
        let start_time = Instant::now();
        let start_path = vfs.canonicalize(start_path)?;
        let start_path = start_path.as_path();

        log::trace!("Starting new ServeSession at path {}", start_path.display());

        let root_project = Project::load_initial_project(&vfs, start_path)?;

        let mut instance_context = InstanceContext::with_emit_legacy_scripts(Some(
            root_project.effective_emit_legacy_scripts(),
        ));
        instance_context.sourcemap_only = sourcemap_only;
        instance_context.offline_session = offline_session;

        log::trace!("Generating snapshot of instances from VFS");
        let snapshot = snapshot_from_vfs(&instance_context, &vfs, start_path)?;

        Self::from_snapshot(vfs, root_project, start_time, snapshot, None)
    }

    pub(crate) fn new_from_stdin(vfs: Vfs, project: Project) -> Result<Self, ServeSessionError> {
        let started = Instant::now();
        let snapshot = Some(crate::builder::stdin_snapshot(&vfs, &project)?);
        let frozen = Arc::new(project.clone());
        Self::from_snapshot(vfs, project, started, snapshot, Some(frozen))
    }

    fn from_snapshot(
        vfs: Vfs,
        root_project: Project,
        start_time: Instant,
        snapshot: Option<InstanceSnapshot>,
        frozen_project: Option<Arc<Project>>,
    ) -> Result<Self, ServeSessionError> {
        crate::sync_authority::check_live_authority(snapshot.as_ref())?;
        let mut tree = RojoTree::new(InstanceSnapshot::new());
        let root_id = tree.get_root_id();
        log::trace!("Computing initial patch set");
        let patch_set = compute_patch_set(snapshot, &tree, root_id);

        log::trace!("Applying initial patch set");
        apply_patch_set(&mut tree, patch_set);

        // Warn once at startup, rather than on every filesystem re-snapshot.
        root_project.warn_duplicate_paths(&vfs);

        let session_id = SessionId::new();
        let message_queue = MessageQueue::new();

        let tree = Arc::new(Mutex::new(tree));
        let message_queue = Arc::new(message_queue);
        let vfs = Arc::new(vfs);

        let (tree_mutation_sender, tree_mutation_receiver) = crossbeam_channel::unbounded();
        let (receipt_sender, receipt_receiver) =
            crossbeam_channel::bounded(crate::write_receipts::PENDING_LIMIT);
        let receipts = Arc::new(Mutex::new(crate::write_receipts::Broker::default()));

        log::trace!("Starting ChangeProcessor");
        let change_processor = ChangeProcessor::start(
            Arc::clone(&tree),
            Arc::clone(&vfs),
            Arc::clone(&message_queue),
            tree_mutation_receiver,
            frozen_project,
            crate::write_receipts::Processor {
                receiver: receipt_receiver,
                session_id,
                broker: Arc::clone(&receipts),
            },
        );

        Ok(Self {
            change_processor,
            start_time,
            session_id,
            root_project,
            tree,
            message_queue,
            tree_mutation_sender,
            receipt_sender,
            receipts,
            vfs,
        })
    }

    pub fn tree(&self) -> MutexGuard<'_, RojoTree> {
        self.tree.lock().unwrap()
    }

    pub fn tree_mutation_sender(&self) -> Sender<PatchSet> {
        self.tree_mutation_sender.clone()
    }

    pub(crate) fn receipt_sender(&self) -> Sender<crate::write_receipts::Command> {
        self.receipt_sender.clone()
    }
    pub(crate) fn receipts(&self) -> crate::write_receipts::Shared {
        Arc::clone(&self.receipts)
    }

    #[allow(unused)]
    pub fn vfs(&self) -> &Vfs {
        &self.vfs
    }

    pub fn message_queue(&self) -> &MessageQueue<AppliedPatchSet> {
        &self.message_queue
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn project_name(&self) -> &str {
        self.root_project
            .name
            .as_ref()
            .expect("all top-level projects must have their name set")
    }

    pub fn project_port(&self) -> Option<u16> {
        self.root_project.serve_port
    }

    pub fn place_id(&self) -> Option<u64> {
        self.root_project.place_id
    }

    pub fn game_id(&self) -> Option<u64> {
        self.root_project.game_id
    }

    pub fn start_time(&self) -> Instant {
        self.start_time
    }

    pub fn serve_place_ids(&self) -> Option<&HashSet<u64>> {
        self.root_project.serve_place_ids.as_ref()
    }

    pub fn blocked_place_ids(&self) -> Option<&HashSet<u64>> {
        self.root_project.blocked_place_ids.as_ref()
    }

    pub fn serve_address(&self) -> Option<IpAddr> {
        self.root_project.serve_address
    }

    pub fn serve_allowed_hosts(&self) -> &[String] {
        &self.root_project.serve_allowed_hosts
    }

    pub fn root_dir(&self) -> &Path {
        self.root_project.folder_location()
    }

    pub fn root_project(&self) -> &Project {
        &self.root_project
    }
}

#[derive(Debug, Error)]
pub enum ServeSessionError {
    #[error(transparent)]
    Io {
        #[from]
        source: io::Error,
    },

    #[error(transparent)]
    Project {
        #[from]
        source: ProjectError,
    },

    #[error(transparent)]
    Other {
        #[from]
        source: anyhow::Error,
    },
}

#[cfg(test)]
mod test {
    use super::*;

    use memofs::StdBackend;

    #[test]
    fn live_authority_rejects_unknown_instance_classes_before_transport() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("default.project.json"), r#"{"name":"Root","syncAuthority":[{"className":"BasePart","properties":"studio"}],"tree":{"$className":"UnknownFutureClass"}}"#).unwrap();
        let error = ServeSession::new(Vfs::new_default().unwrap(), directory.path())
            .err()
            .expect("unknown authority class was accepted");
        assert!(
            error.to_string().contains("Unknown managed instance class"),
            "{error}"
        );
    }

    #[test]
    fn studio_property_authority_supports_live_sessions_with_filesystem_structure() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("default.project.json"), r#"{"name":"Root","syncAuthority":[{"className":"BasePart","properties":"studio"}],"tree":{"$className":"Folder","Pole":{"$className":"Part"}}}"#).unwrap();
        assert!(ServeSession::new(Vfs::new_default().unwrap(), directory.path()).is_ok());
    }

    #[test]
    fn staged_non_filesystem_authority_allows_offline_build_but_refuses_live_session() {
        use memofs::{InMemoryFs, VfsSnapshot};
        use serde_json::json;
        for policy in [
            json!({"className":"Part","properties":"twoWay"}),
            json!({"className":"Part","properties":"filesystem","structure":"twoWay"}),
        ] {
            let mut fs = InMemoryFs::new();
            fs.load_snapshot("/project", VfsSnapshot::dir([("default.project.json", VfsSnapshot::file(json!({"name":"Root","syncAuthority":[policy],"tree":{"$className":"Folder","Pole":{"$className":"Part"}}}).to_string()))])).unwrap();
            let vfs = Vfs::new(fs.clone());
            let project =
                Project::load_exact(&vfs, "/project/default.project.json".as_ref(), None).unwrap();
            assert!(crate::build_project(&vfs, &project).is_ok());
            let error = match ServeSession::new(vfs, "/project/default.project.json") {
                Ok(_) => panic!("Unimplemented live authority was accepted"),
                Err(error) => error,
            };
            assert!(
                error.to_string().contains("reviewed client authority"),
                "{error}"
            );
            let sourcemap =
                ServeSession::new_for_sourcemap(Vfs::new(fs), "/project/default.project.json")
                    .unwrap();
            assert_eq!(sourcemap.tree().root().name(), "Root");
        }
    }

    #[test]
    fn tree_is_keyed_by_canonical_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("default.project.json"),
            r#"{ "name": "test", "tree": { "$className": "Folder" } }"#,
        )
        .unwrap();

        // `std::fs::canonicalize` yields a verbatim path on Windows.
        // On other platforms it simply resolves the path (e.g. symlinks),
        // which the session must also handle.
        let start_path = std::fs::canonicalize(dir.path()).unwrap();

        let vfs = Vfs::new(StdBackend::new().unwrap());
        let session = ServeSession::new(vfs, &start_path).unwrap();

        let project_file = start_path.join("default.project.json");
        let canonical = session.vfs().canonicalize(&project_file).unwrap();

        assert!(
            !session.tree().get_ids_at_path(&canonical).is_empty(),
            "project file {} should be tracked in the tree under its canonical \
             path {}, matching what the watcher reports",
            project_file.display(),
            canonical.display(),
        );
    }
}
