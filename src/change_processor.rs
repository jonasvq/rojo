use anyhow::Context as _;
use crossbeam_channel::{select, Receiver, RecvError, Sender};
use jod_thread::JoinHandle;
use memofs::{IoResultExt, Vfs, VfsEvent};
use rbx_dom_weak::types::{Ref, Variant};
use std::path::{Path, PathBuf};
use std::{
    collections::HashSet,
    fs, io,
    sync::{Arc, Mutex},
};

use crate::{
    message_queue::MessageQueue,
    snapshot::{
        apply_patch_set, compute_patch_set, AppliedPatchSet, InstanceMetadata, InstigatingSource,
        PatchSet, RojoTree,
    },
    snapshot_middleware::{
        get_dir_middleware, snapshot_from_vfs, snapshot_project_node, Middleware,
    },
};

/// Processes file change events, updates the DOM, and sends those updates
/// through a channel for other stuff to consume.
///
/// Owns the connection between Rojo's VFS and its DOM by holding onto another
/// thread that processes messages.
///
/// Consumers of ChangeProcessor, like ServeSession, are intended to communicate
/// with this object via channels.
///
/// ChangeProcessor expects to be the only writer to the RojoTree and Vfs
/// objects passed to it.
pub struct ChangeProcessor {
    /// Controls the runtime of the processor thread. When signaled, the job
    /// thread will finish its current work and terminate.
    ///
    /// This channel should be signaled before dropping ChangeProcessor or we'll
    /// hang forever waiting for the message processing loop to terminate.
    shutdown_sender: Sender<()>,

    /// A handle to the message processing thread. When dropped, we'll block
    /// until it's done.
    ///
    /// Allowed to be unused because dropping this value has side effects.
    #[allow(unused)]
    job_thread: JoinHandle<Result<(), RecvError>>,
}

impl ChangeProcessor {
    /// Spin up the ChangeProcessor, connecting it to the given tree, VFS, and
    /// outbound message queue.
    pub(crate) fn start(
        tree: Arc<Mutex<RojoTree>>,
        vfs: Arc<Vfs>,
        message_queue: Arc<MessageQueue<AppliedPatchSet>>,
        tree_mutation_receiver: Receiver<PatchSet>,
        frozen_project: Option<Arc<crate::Project>>,
        receipt_processor: crate::write_receipts::Processor,
    ) -> Self {
        let (shutdown_sender, shutdown_receiver) = crossbeam_channel::bounded(1);
        let vfs_receiver = vfs.event_receiver();
        let task = JobThreadContext {
            tree,
            vfs,
            message_queue,
            frozen_project,
        };

        let job_thread = jod_thread::Builder::new()
            .name("ChangeProcessor thread".to_owned())
            .spawn(move || {
                let _receipts_guard = crate::write_receipts::ProcessorGuard(Arc::clone(&receipt_processor.broker));
                log::trace!("ChangeProcessor thread started");

                loop {
                    select! {
                        recv(vfs_receiver) -> event => {
                            task.handle_vfs_event(event?);
                        },
                        recv(tree_mutation_receiver) -> patch_set => {
                            task.handle_tree_event(patch_set?);
                        },
                        recv(shutdown_receiver) -> _ => {
                            log::trace!("ChangeProcessor shutdown signal received...");
                            return Ok(());
                        },
                        recv(receipt_processor.receiver) -> command => {
                            task.handle_receipt(command?, receipt_processor.session_id, &receipt_processor.broker);
                        },
                    }
                }
            })
            .expect("Could not start ChangeProcessor thread");

        Self {
            shutdown_sender,
            job_thread,
        }
    }
}

impl Drop for ChangeProcessor {
    fn drop(&mut self) {
        // Signal the job thread to start spinning down. Without this we'll hang
        // forever waiting for the thread to finish its infinite loop.
        let _ = self.shutdown_sender.send(());

        // After this function ends, the job thread will be joined. It might
        // block for a small period of time while it processes its last work.
    }
}

/// Contains all of the state needed to synchronize the DOM and VFS.
struct JobThreadContext {
    /// A handle to the DOM we're managing.
    tree: Arc<Mutex<RojoTree>>,

    /// A handle to the VFS we're managing.
    vfs: Arc<Vfs>,

    /// Whenever changes are applied to the DOM, we should push those changes
    /// into this message queue to inform any connected clients.
    message_queue: Arc<MessageQueue<AppliedPatchSet>>,
    frozen_project: Option<Arc<crate::Project>>,
}

impl JobThreadContext {
    fn refresh_receipt_tree(&self) -> bool {
        let patch = {
            let mut tree = self.tree.lock().unwrap();
            let root = tree.get_root_id();
            compute_and_apply_changes(&mut tree, &self.vfs, root, self.frozen_project.as_deref())
        };
        if let Some(patch) = patch {
            if !patch.is_empty() {
                self.message_queue.push_messages(&[patch]);
            }
            true
        } else {
            false
        }
    }

    fn handle_receipt(
        &self,
        command: crate::write_receipts::Command,
        session: crate::SessionId,
        broker: &crate::write_receipts::Shared,
    ) {
        use crate::write_receipts::{read_source, Command, Outcome, State};
        let refreshed = self.refresh_receipt_tree();
        match command {
            Command::Read(id, sender) => {
                let result = if refreshed {
                    read_source(session, &self.tree.lock().unwrap(), &self.vfs, id)
                        .map(|source| source.view)
                } else {
                    Err(anyhow::anyhow!("Project refresh failed"))
                };
                if let Err(error) = &result {
                    log::warn!("Receipt source binding unavailable: {error:#}");
                }
                let _ = sender.send(result);
            }
            Command::Write(id, request) => {
                let outcome = if !refreshed {
                    Outcome::new(
                        State::Rejected,
                        "Project refresh failed; no receipt write was attempted",
                    )
                } else {
                    let source = read_source(
                        session,
                        &self.tree.lock().unwrap(),
                        &self.vfs,
                        request.instance_id,
                    );
                    match source {
                        Err(error) => {
                            log::warn!("Receipt binding rejected: {error:#}");
                            Outcome::new(
                                State::Rejected,
                                "Source binding is missing, changed, ambiguous or not reversible",
                            )
                        }
                        Ok(source) => 'source_request: {
                            let expected = match source.requested_source(&request.source) {
                                Ok(expected) => expected,
                                Err(_) => break 'source_request Outcome::new(State::Rejected, "textModule Source is not a canonical data literal; no write was attempted").observed(&source.view, self.message_queue.cursor()),
                            };
                            if source.view.class_name != request.class_name
                                || source.view.binding_hash != request.binding_hash
                            {
                                Outcome::new(
                                    State::Conflict,
                                    "Source binding or class changed; review before writing",
                                )
                                .observed(&source.view, self.message_queue.cursor())
                            } else if source.view.source == expected {
                                Outcome::new(
                                    State::Unchanged,
                                    "Source already matches; no write needed",
                                )
                                .observed(&source.view, self.message_queue.cursor())
                            } else if source.view.source_hash != request.source_hash {
                                Outcome::new(
                                    State::Conflict,
                                    "Source changed since the accepted base; review before writing",
                                )
                                .observed(&source.view, self.message_queue.cursor())
                            } else {
                                let write = source.write(&self.vfs, &request.source);
                                let refreshed = self.refresh_receipt_tree();
                                let mut result = if let Err(error) = &write {
                                    log::warn!("Receipt source write failed: {error:#}");
                                    Outcome::new(State::IoError, "Could not write source; inspect the file and serve terminal before retrying")
                                } else {
                                    Outcome::new(State::Uncertain, "Source was written but authoritative readback could not be confirmed; inspect before retrying")
                                };
                                result.may_have_changed = true;
                                if refreshed {
                                    if let Ok(after) = read_source(
                                        session,
                                        &self.tree.lock().unwrap(),
                                        &self.vfs,
                                        request.instance_id,
                                    ) {
                                        if write.is_ok() {
                                            result = if after.view.source == expected
                                                && after.view.binding_hash == request.binding_hash
                                            {
                                                Outcome::new(
                                                    State::Applied,
                                                    "Source persistence and readback confirmed",
                                                )
                                            } else {
                                                Outcome::new(State::Conflict, "Source or binding changed during write/readback; review current state")
                                            };
                                            result.may_have_changed = true;
                                        }
                                        result = result
                                            .observed(&after.view, self.message_queue.cursor());
                                    }
                                }
                                result
                            }
                        }
                    }
                };
                broker.lock().unwrap().complete(id, outcome);
            }
        }
    }
    /// Computes and applies patches to the DOM for a given file path.
    ///
    /// This function finds the nearest ancestor to the given path that has associated instances
    /// in the tree.
    /// It then computes and applies changes for each affected instance ID and
    /// returns a vector of applied patch sets.
    fn apply_patches(&self, path: PathBuf) -> Vec<AppliedPatchSet> {
        let mut tree = self.tree.lock().unwrap();
        let mut applied_patches = Vec::new();

        // Find the nearest ancestor to this path that has
        // associated instances in the tree. This helps make sure
        // that we handle additions correctly, especially if we
        // receive events for descendants of a large tree being
        // created all at once.
        let mut current_path = path.as_path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                crate::project::INIT_PROJECT_NAMES
                    .iter()
                    .any(|entry| entry.eq_ignore_ascii_case(name))
            })
        {
            if let Some(parent) = path.parent() {
                // A V1 child project can become its directory's V2 entry (or
                // vice versa). Refresh the containing binding, not just the
                // previously named child. Explicit file roots have no such
                // directory binding and keep their original refresh path.
                if !tree.get_ids_at_path(parent).is_empty() {
                    current_path = parent;
                }
            }
        }
        let mut affected_ids = loop {
            let ids = tree.get_ids_at_path(current_path);

            log::trace!("Path {} affects IDs {:?}", current_path.display(), ids);

            if !ids.is_empty() {
                break ids.to_vec();
            }

            log::trace!("Trying parent path...");
            match current_path.parent() {
                Some(parent) => current_path = parent,
                None => break Vec::new(),
            }
        };

        // Last-file removal can make an existing implicit ancestor disappear.
        // Re-evaluate its enclosing explicit/init/project boundary, rather than
        // only updating the leaf and leaving empty Folders in the live tree.
        for id in &mut affected_ids {
            let Some(metadata) = tree.get_metadata(*id) else {
                continue;
            };
            if !metadata.context.ignore_empty_directories
                || !matches!(
                    metadata.instigating_source,
                    Some(InstigatingSource::Path(_))
                )
                || metadata.middleware == Some(crate::Middleware::Project)
            {
                continue;
            }
            if let Some(InstigatingSource::Path(source)) = &metadata.instigating_source {
                // A surviving file cannot make its implicit ancestors empty.
                if self.vfs.metadata(source).is_ok_and(|meta| meta.is_file()) {
                    continue;
                }
            }
            let mut current = *id;
            if metadata.middleware != Some(crate::Middleware::Dir) {
                let Some(instance) = tree.get_instance(current) else {
                    continue;
                };
                current = instance.parent();
            }
            while let Some(instance) = tree.get_instance(current) {
                let current_meta = instance.metadata();
                if current_meta.middleware != Some(crate::Middleware::Dir)
                    || !matches!(
                        current_meta.instigating_source,
                        Some(InstigatingSource::Path(_))
                    )
                    || !current_meta.context.ignore_empty_directories
                {
                    break;
                }
                let parent = instance.parent();
                if tree.get_metadata(parent).is_none() {
                    break;
                }
                *id = parent;
                current = parent;
            }
        }
        let mut seen = HashSet::new();
        affected_ids.retain(|id| seen.insert(*id));
        affected_ids.retain(|id| {
            let mut current = *id;
            while let Some(instance) = tree.get_instance(current) {
                current = instance.parent();
                if seen.contains(&current) {
                    return false;
                }
            }
            true
        });

        // A merged container can lose one contributing directory, or gain a
        // collision in another. Rebuild its enclosing project rather than
        // treating the missing contribution as the entire merged instance.
        let merged_ancestor = affected_ids.iter().any(|id| {
            if !tree
                .get_metadata(*id)
                .is_some_and(|metadata| metadata.context.sourcemap_only)
            {
                return false;
            }
            let mut current = *id;
            while let Some(instance) = tree.get_instance(current) {
                if instance.metadata().context.sourcemap_only
                    && matches!(&instance.metadata().instigating_source,
                        Some(InstigatingSource::ProjectNode { node, .. })
                        if matches!(node.path, Some(crate::PathNode::Multiple(_))))
                {
                    return true;
                }
                current = instance.parent();
            }
            false
        });
        if merged_ancestor {
            affected_ids = vec![tree.get_root_id()];
        }

        for id in affected_ids {
            if let Some(patch) =
                compute_and_apply_changes(&mut tree, &self.vfs, id, self.frozen_project.as_deref())
            {
                if !patch.is_empty() {
                    applied_patches.push(patch);
                }
            }
        }

        applied_patches
    }

    fn handle_vfs_event(&self, event: VfsEvent) {
        let _operation = match &event {
            VfsEvent::Create(path) => Some(crate::diagnostic_context::enter(
                "processing filesystem create",
                path,
            )),
            VfsEvent::Write(path) => Some(crate::diagnostic_context::enter(
                "processing filesystem write",
                path,
            )),
            VfsEvent::Remove(path) => Some(crate::diagnostic_context::enter(
                "processing filesystem remove",
                path,
            )),
            _ => None,
        };
        log::trace!("Vfs event: {:?}", event);

        // Update the VFS immediately with the event.
        if let Err(err) = self.vfs.commit_event(&event) {
            log::error!("Could not commit filesystem event {:?}: {}", event, err);
            return;
        }

        // For a given VFS event, we might have many changes to different parts
        // of the tree. Calculate and apply all of these changes.
        let applied_patches = match event {
            VfsEvent::Create(path) | VfsEvent::Write(path) | VfsEvent::Remove(path) => {
                match canonicalize_event_path(&self.vfs, &path) {
                    Ok(path) => self.apply_patches(path),
                    Err(err) => {
                        log::error!(
                            "Could not resolve filesystem event path {}: {}",
                            path.display(),
                            err
                        );
                        Vec::new()
                    }
                }
            }
            _ => {
                log::warn!("Unhandled VFS event: {:?}", event);
                Vec::new()
            }
        };

        // Notify anyone listening to the message queue about the changes we
        // just made.
        self.message_queue.push_messages(&applied_patches);
    }

    fn handle_tree_event(&self, mut patch_set: PatchSet) {
        log::trace!("Applying PatchSet from client: {:#?}", patch_set);

        {
            let tree = self.tree.lock().unwrap();
            let fresh = compute_changes(
                &tree,
                &self.vfs,
                tree.get_root_id(),
                self.frozen_project.as_deref(),
            );
            if fresh.is_none_or(|patch| {
                !patch.added_instances.is_empty()
                    || !patch.removed_instances.is_empty()
                    || !patch.updated_instances.is_empty()
            }) {
                // Legacy requests carry no accepted binding. Wait for the real
                // filesystem update instead of granting rights from a stale
                // tree or publishing an unrelated patch as a rejected write.
                log::warn!("Project changed or refresh failed; legacy client write requires a current filesystem baseline");
                return;
            }
        }

        let applied_patch = {
            let mut tree = self.tree.lock().unwrap();

            patch_set.removed_instances.retain(|id| {
                if tree.get_instance(*id).is_some() {
                    for instance in tree.descendants(*id) {
                        if let Err(error) = crate::write_receipts::ensure_legacy_write_authority(
                            instance.metadata(),
                            instance.class_name().as_str(),
                        ) {
                            log::warn!("Legacy removal refused: {error:#}");
                            return false;
                        }
                    }
                }
                true
            });

            patch_set.added_instances.retain(|addition| {
                let Some(parent) = tree.get_instance(addition.parent_id) else {
                    log::warn!("Legacy addition refused: managed parent is missing");
                    return false;
                };
                if let Err(error) = crate::write_receipts::ensure_legacy_write_authority(
                    parent.metadata(),
                    parent.class_name().as_str(),
                ) {
                    log::warn!("Legacy addition refused: {error:#}");
                    return false;
                }
                // Never trust client-supplied metadata to clear inherited class
                // authority on newly added descendants.
                let mut pending = vec![&addition.instance];
                while let Some(node) = pending.pop() {
                    if let Err(error) = crate::write_receipts::ensure_legacy_write_authority(
                        parent.metadata(),
                        node.class_name.as_str(),
                    ) {
                        log::warn!("Legacy addition refused: {error:#}");
                        return false;
                    }
                    pending.extend(&node.children);
                }
                true
            });

            for &id in &patch_set.removed_instances {
                if let Some(instance) = tree.get_instance(id) {
                    if let Some(instigating_source) = &instance.metadata().instigating_source {
                        match instigating_source {
                            InstigatingSource::Path(path) => {
                                if let Err(err) = fs::remove_file(path) {
                                    log::error!(
                                        "Failed to remove file {}: {}",
                                        path.display(),
                                        err
                                    );
                                }
                            }
                            InstigatingSource::ProjectNode { .. } => {
                                log::warn!(
                                    "Cannot remove instance {:?}, it's from a project file",
                                    id
                                );
                            }
                        }
                    } else {
                        // TODO
                        log::warn!(
                            "Cannot remove instance {:?}, it is not an instigating source.",
                            id
                        );
                    }
                } else {
                    log::warn!("Cannot remove instance {:?}, it does not exist.", id);
                }
            }

            for update in &mut patch_set.updated_instances {
                let id = update.id;

                if let Some(instance) = tree.get_instance(id) {
                    if update.changed_name.is_some() {
                        log::warn!("Cannot rename instances yet.");
                    }

                    if update.changed_class_name.is_some() {
                        log::warn!("Cannot change ClassName yet.");
                    }

                    if update.changed_metadata.is_some() {
                        log::warn!("Cannot change metadata yet.");
                    }

                    update.changed_properties.retain(|key, changed_value| {
                        if let Err(error) = crate::write_receipts::ensure_legacy_write_authority(
                            instance.metadata(),
                            instance.class_name().as_str(),
                        ) {
                            log::warn!("Legacy client write refused: {error:#}");
                            return false;
                        }
                        if key != "Source" {
                            log::warn!("Cannot change properties besides BaseScript.Source.");
                            return false;
                        }
                        let Some(Variant::String(value)) = changed_value else {
                            log::warn!("Cannot change Source to non-string value.");
                            return false;
                        };
                        let result =
                            script_source_path(&self.vfs, instance.metadata()).and_then(|path| {
                                let _operation = crate::diagnostic_context::enter(
                                    "writing script source",
                                    &path,
                                );
                                let contents = if instance.metadata().middleware
                                    == Some(Middleware::TextModule)
                                {
                                    std::borrow::Cow::Owned(
                                        crate::snapshot_middleware::decode_text_module(value)
                                            .with_context(|| {
                                                format!(
                                                    "Cannot write textModule {}",
                                                    path.display()
                                                )
                                            })?,
                                    )
                                } else {
                                    std::borrow::Cow::Borrowed(value.as_str())
                                };
                                self.vfs.write(&path, contents.as_bytes()).with_context(|| {
                                    format!("Could not write script source {}", path.display())
                                })
                            });
                        match result {
                            Ok(()) => true,
                            Err(err) => {
                                log::error!(
                                    "Could not update Source for instance {:?}: {:#}",
                                    id,
                                    err
                                );
                                false
                            }
                        }
                    });
                } else {
                    log::warn!("Cannot update instance {:?}, it does not exist.", id);
                    update.changed_properties.clear();
                }
                update.changed_name = None;
                update.changed_class_name = None;
                update.changed_metadata = None;
            }

            // Publishing a failed write makes the server and plugin agree on a
            // value that was never persisted, hiding the original filesystem error.
            patch_set
                .updated_instances
                .retain(|update| !update.changed_properties.is_empty());

            apply_patch_set(&mut tree, patch_set)
        };

        if !applied_patch.is_empty() {
            self.message_queue.push_messages(&[applied_patch]);
        }
    }
}

pub(crate) fn script_source_path(
    vfs: &Vfs,
    metadata: &InstanceMetadata,
) -> anyhow::Result<PathBuf> {
    let path = match &metadata.instigating_source {
        Some(InstigatingSource::Path(path)) => path.clone(),
        Some(InstigatingSource::ProjectNode { path, node, .. }) => {
            let source = node
                .path
                .as_ref()
                .context("Project node has no source path")?
                .path();
            if source.is_absolute() {
                source.to_path_buf()
            } else {
                path.parent()
                    .context("Project file has no parent directory")?
                    .join(source)
            }
        }
        None => anyhow::bail!("Instance has no source path"),
    };
    match metadata.middleware {
        Some(
            Middleware::ServerScriptDir
            | Middleware::ClientScriptDir
            | Middleware::ModuleScriptDir
            | Middleware::PluginScriptDir,
        ) => {
            let (current, _, init_path) = get_dir_middleware(vfs, &path)?;
            anyhow::ensure!(
                Some(current) == metadata.middleware,
                "Script source directory {} changed type",
                path.display()
            );
            Ok(init_path)
        }
        Some(
            Middleware::ServerScript
            | Middleware::ClientScript
            | Middleware::ModuleScript
            | Middleware::PluginScript
            | Middleware::LegacyServerScript
            | Middleware::LegacyClientScript
            | Middleware::RunContextServerScript
            | Middleware::RunContextClientScript
            | Middleware::TextModule,
        ) => {
            anyhow::ensure!(
                vfs.metadata(&path)?.is_file(),
                "Script source {} is no longer a file",
                path.display()
            );
            Ok(path)
        }
        _ => anyhow::bail!(
            "Source at {} is generated and cannot be written as Luau",
            path.display()
        ),
    }
}

fn canonicalize_event_path(vfs: &Vfs, path: &Path) -> io::Result<PathBuf> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match vfs.canonicalize(ancestor) {
            Ok(mut normalized) => {
                // Keep the missing suffix so that removed sources still resolve to
                // their tracked IDs, including when several parents disappeared.
                for component in missing.into_iter().rev() {
                    normalized.push(component);
                }
                return Ok(normalized);
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                match (ancestor.parent(), ancestor.file_name()) {
                    (Some(parent), Some(name)) => {
                        missing.push(name.to_os_string());
                        ancestor = parent;
                    }
                    _ => return Err(err),
                }
            }
            Err(err) => return Err(err),
        }
    }
}

fn compute_and_apply_changes(
    tree: &mut RojoTree,
    vfs: &Vfs,
    id: Ref,
    frozen_project: Option<&crate::Project>,
) -> Option<AppliedPatchSet> {
    let patch = compute_changes(tree, vfs, id, frozen_project)?;
    Some(apply_patch_set(tree, patch))
}

fn compute_changes(
    tree: &RojoTree,
    vfs: &Vfs,
    id: Ref,
    frozen_project: Option<&crate::Project>,
) -> Option<PatchSet> {
    let metadata = tree
        .get_metadata(id)
        .expect("metadata missing for instance present in tree");

    let instigating_source = match &metadata.instigating_source {
        Some(path) => path,
        None => {
            log::error!(
                "Instance {:?} did not have an instigating source, but was considered for an update.",
                id
            );
            log::error!("This is a bug. Please file an issue!");
            return None;
        }
    };

    // How we process a file change event depends on what created this
    // file/folder in the first place.
    let applied_patch_set = match instigating_source {
        InstigatingSource::Path(path)
            if frozen_project.is_some_and(|project| &project.file_location == path) =>
        {
            let project = frozen_project.unwrap();
            let snapshot = match crate::snapshot_middleware::snapshot_stdin_project(
                &metadata.context,
                vfs,
                project,
            ) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    log::error!("Could not refresh stdin project: {error:#}");
                    return None;
                }
            };
            checked_live_patch(snapshot, tree, id)?
        }
        InstigatingSource::Path(path) => match vfs.metadata(path).with_not_found() {
            Ok(Some(_)) => {
                // Our instance was previously created from a path and that
                // path still exists. We can generate a snapshot starting at
                // that path and use it as the source for our patch.

                let snapshot = match snapshot_from_vfs(&metadata.context, vfs, path) {
                    Ok(snapshot) => snapshot,
                    Err(err) => {
                        log::error!("Snapshot error: {:?}", err);
                        return None;
                    }
                };

                checked_live_patch(snapshot, tree, id)?
            }
            Ok(None) => {
                if id == tree.get_root_id() {
                    // Atomic saves can temporarily remove the project/root source.
                    // Keep its path associations so recreation can restore syncing.
                    log::warn!(
                        "Root source {} is missing; keeping the last valid tree",
                        path.display()
                    );
                    return None;
                }
                // Our instance was previously created from a path, but that
                // path no longer exists.
                //
                // We associate deleting the instigating file for an
                // instance with deleting that instance.

                let mut patch_set = PatchSet::new();
                patch_set.removed_instances.push(id);

                patch_set
            }
            Err(err) => {
                log::error!("Error processing filesystem change: {:?}", err);
                return None;
            }
        },

        InstigatingSource::ProjectNode {
            path,
            name,
            node,
            parent_class,
        } => {
            // This instance is the direct subject of a project node. Since
            // there might be information associated with our instance from
            // the project file, we snapshot the entire project node again.

            let snapshot_result = snapshot_project_node(
                &metadata.context,
                path,
                name,
                node,
                vfs,
                parent_class.as_ref().map(|name| name.as_str()),
            );

            let snapshot = match snapshot_result {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    log::error!("{:?}", err);
                    return None;
                }
            };

            checked_live_patch(snapshot, tree, id)?
        }
    };

    Some(applied_patch_set)
}

fn checked_live_patch(
    snapshot: Option<crate::InstanceSnapshot>,
    tree: &RojoTree,
    id: Ref,
) -> Option<PatchSet> {
    if let Err(error) = crate::sync_authority::check_live_authority(snapshot.as_ref()) {
        log::error!("Live authority unavailable; keeping the last valid tree: {error:#}");
        return None;
    }
    Some(compute_patch_set(snapshot, tree, id))
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::snapshot::InstanceContext;
    use memofs::{InMemoryFs, VfsSnapshot};

    fn source_update(id: Ref, value: &str) -> PatchSet {
        PatchSet {
            updated_instances: vec![crate::snapshot::PatchUpdate {
                id,
                changed_name: None,
                changed_class_name: None,
                changed_metadata: None,
                changed_properties: [(
                    rbx_dom_weak::ustr("Source"),
                    Some(Variant::String(value.into())),
                )]
                .into_iter()
                .collect(),
            }],
            ..PatchSet::new()
        }
    }

    #[test]
    fn custom_csv_route_keeps_context_and_identity_on_direct_watch_updates() {
        let task = fixture();
        let path = "/project/src/Table.loc";
        task.vfs.write(path, "Key,Source\nhello,before\n").unwrap();
        task.vfs.write("/project/default.project.json", r#"{"name":"Root","syncRules":[{"pattern":"*.loc","use":"csv","suffix":".loc"}],"tree":{"$path":"src"}}"#).unwrap();
        let snapshot = snapshot_from_vfs(
            &InstanceContext::default(),
            &task.vfs,
            "/project/default.project.json".as_ref(),
        )
        .unwrap()
        .unwrap();
        *task.tree.lock().unwrap() = RojoTree::new(snapshot);
        let id = task.tree.lock().unwrap().get_ids_at_path(path.as_ref())[0];
        task.vfs.write(path, "Key,Source\nhello,after\n").unwrap();
        task.handle_vfs_event(VfsEvent::Write(path.into()));
        let tree = task.tree.lock().unwrap();
        let table = tree
            .get_instance(id)
            .expect("CSV watch removed the mapped table");
        let Some(Variant::String(contents)) = table.properties().get(&"Contents".into()) else {
            panic!("CSV contents are missing");
        };
        assert!(contents.contains("after"));
    }

    #[test]
    fn explicit_authority_blocks_entire_legacy_updates_and_removals() {
        for operation in 0..4 {
            let task = fixture();
            let project_path = "/project/default.project.json";
            task.vfs.write(project_path, r#"{"name":"Root","syncAuthority":[{"className":"LuaSourceContainer","properties":"filesystem"}],"tree":{"$className":"Folder","Group":{"$className":"Folder","Child":{"$path":"src/nested/child.luau"}}}}"#).unwrap();
            let snapshot = snapshot_from_vfs(
                &InstanceContext::default(),
                &task.vfs,
                project_path.as_ref(),
            )
            .unwrap()
            .unwrap();
            *task.tree.lock().unwrap() = RojoTree::new(snapshot);
            let id = task
                .tree
                .lock()
                .unwrap()
                .get_ids_at_path("/project/src/nested/child.luau".as_ref())[0];
            let mut patch = source_update(id, "return 'forbidden'");
            let parent = task.tree.lock().unwrap().get_instance(id).unwrap().parent();
            match operation {
                0 => patch.updated_instances[0].changed_name = Some("Renamed".into()),
                1 | 2 => {
                    patch.updated_instances.clear();
                    patch
                        .removed_instances
                        .push(if operation == 1 { id } else { parent });
                }
                _ => {
                    patch.updated_instances.clear();
                    patch.added_instances.push(crate::snapshot::PatchAdd {
                        parent_id: parent,
                        instance: crate::InstanceSnapshot::new()
                            .class_name("Folder")
                            .name("Injected")
                            .children(vec![crate::InstanceSnapshot::new()
                                .class_name("ModuleScript")
                                .name("Protected")]),
                    });
                }
            }
            task.handle_tree_event(patch);
            assert_eq!(
                task.tree.lock().unwrap().get_instance(id).unwrap().name(),
                "Child"
            );
            assert_eq!(
                task.vfs
                    .read_to_string("/project/src/nested/child.luau")
                    .unwrap()
                    .as_str(),
                "return 1"
            );
            assert_eq!(
                task.tree
                    .lock()
                    .unwrap()
                    .get_instance(parent)
                    .unwrap()
                    .children(),
                &[id]
            );
        }
    }

    #[test]
    fn source_authority_revocation_is_checked_before_legacy_writes_without_watch_events() {
        let task = fixture();
        let project_path = "/project/default.project.json";
        let project = r#"{"name":"Root","tree":{"$className":"Folder","Child":{"$path":"src/nested/child.luau"}}}"#;
        task.vfs.write(project_path, project).unwrap();
        let snapshot = snapshot_from_vfs(
            &InstanceContext::default(),
            &task.vfs,
            project_path.as_ref(),
        )
        .unwrap()
        .unwrap();
        *task.tree.lock().unwrap() = RojoTree::new(snapshot);
        let id = task
            .tree
            .lock()
            .unwrap()
            .get_ids_at_path("/project/src/nested/child.luau".as_ref())[0];
        task.vfs.write(project_path, r#"{"name":"Root","syncAuthority":[{"className":"LuaSourceContainer","properties":"filesystem"}],"tree":{"$className":"Folder","Child":{"$path":"src/nested/child.luau"}}}"#).unwrap();
        task.handle_tree_event(source_update(id, "return 'forbidden'"));
        assert_eq!(
            task.vfs
                .read_to_string("/project/src/nested/child.luau")
                .unwrap()
                .as_str(),
            "return 1"
        );
        task.vfs.write(project_path, project).unwrap();
        task.handle_tree_event(source_update(id, "return 2"));
        assert_eq!(
            task.vfs
                .read_to_string("/project/src/nested/child.luau")
                .unwrap()
                .as_str(),
            "return 2"
        );
    }

    #[test]
    fn receipt_readback_detects_external_replacement_and_binding_loss_after_write() {
        use crate::write_receipts::{read_source, Broker, State, WriteRequest, AFTER_WRITE};
        for remove in [false, true] {
            let task = fixture();
            let path = "/project/src/nested/child.luau";
            let session = crate::SessionId::new();
            let id = task.tree.lock().unwrap().get_ids_at_path(path.as_ref())[0];
            let source = read_source(session, &task.tree.lock().unwrap(), &task.vfs, id).unwrap();
            let broker = Arc::new(Mutex::new(Broker::default()));
            let (sender, receiver) = crossbeam_channel::bounded(1);
            let request = WriteRequest {
                session_id: session,
                origin_id: uuid::Uuid::new_v4(),
                request_id: uuid::Uuid::new_v4(),
                instance_id: id,
                class_name: source.view.class_name,
                source_hash: source.view.source_hash,
                binding_hash: source.view.binding_hash,
                source: "return 2".into(),
            };
            let receipt = broker
                .lock()
                .unwrap()
                .admit(request, &sender, std::time::Instant::now())
                .unwrap();
            let vfs = Arc::clone(&task.vfs);
            AFTER_WRITE.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    if remove {
                        vfs.remove_file(path).unwrap();
                    } else {
                        vfs.write(path, "return 3").unwrap();
                    }
                }))
            });
            task.handle_receipt(receiver.recv().unwrap(), session, &broker);
            let result = broker.lock().unwrap().test_outcome(receipt.receipt_id);
            assert_eq!(
                result.state,
                if remove {
                    State::Uncertain
                } else {
                    State::Conflict
                }
            );
            assert!(result.may_have_changed);
            if !remove {
                assert_eq!(task.vfs.read_to_string(path).unwrap().as_str(), "return 3");
                assert_eq!(
                    task.tree
                        .lock()
                        .unwrap()
                        .get_instance(id)
                        .unwrap()
                        .properties()
                        .get(&"Source".into()),
                    Some(&Variant::String("return 3".into()))
                );
            }
        }
    }

    #[test]
    fn client_source_updates_write_each_directory_init_script() {
        for name in [
            "init.luau",
            "init.lua",
            "init.server.luau",
            "init.server.lua",
            "init.client.luau",
            "init.client.lua",
            "init.plugin.lua",
            "init.plugin.luau",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join(name);
            fs::write(&script, "return 1").unwrap();
            let vfs = Arc::new(Vfs::new_default().unwrap());
            vfs.set_watch_enabled(false);
            let path = vfs.canonicalize(dir.path()).unwrap();
            let snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, &path)
                .unwrap()
                .unwrap();
            let task = JobThreadContext {
                tree: Arc::new(Mutex::new(RojoTree::new(snapshot))),
                vfs,
                message_queue: Arc::new(MessageQueue::new()),
                frozen_project: None,
            };
            let id = task.tree.lock().unwrap().get_root_id();
            task.handle_tree_event(source_update(id, "return 2"));
            assert_eq!(
                fs::read_to_string(&script).unwrap(),
                "return 2",
                "source update for {name}"
            );
        }
    }

    #[test]
    fn rejected_source_write_does_not_advance_tree_or_publish_patch() {
        let task = fixture();
        let path = "/project/src/nested/child.luau";
        let id = task.tree.lock().unwrap().get_ids_at_path(path.as_ref())[0];
        task.vfs.remove_file(path).unwrap();
        task.vfs.create_dir(path).unwrap();
        task.handle_tree_event(source_update(id, "return 2"));
        let tree = task.tree.lock().unwrap();
        assert_eq!(
            tree.get_instance(id)
                .unwrap()
                .properties()
                .get(&rbx_dom_weak::ustr("Source")),
            Some(&Variant::String("return 1".into()))
        );
        assert_eq!(task.message_queue.cursor(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn read_only_source_write_keeps_state_and_recovers_after_permissions_restore() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("locked.luau");
        fs::write(&script, "return 1").unwrap();
        let original_permissions = fs::metadata(&script).unwrap().permissions();
        let vfs = Arc::new(Vfs::new_default().unwrap());
        vfs.set_watch_enabled(false);
        let path = vfs.canonicalize(&script).unwrap();
        let snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, &path)
            .unwrap()
            .unwrap();
        let task = JobThreadContext {
            tree: Arc::new(Mutex::new(RojoTree::new(snapshot))),
            vfs,
            message_queue: Arc::new(MessageQueue::new()),
            frozen_project: None,
        };
        let id = task.tree.lock().unwrap().get_root_id();
        let mut read_only = original_permissions.clone();
        read_only.set_readonly(true);
        fs::set_permissions(&script, read_only).unwrap();
        let denied = fs::write(&script, "return 2").unwrap_err();
        assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
        task.handle_tree_event(source_update(id, "return 2"));
        fs::set_permissions(&script, original_permissions).unwrap();
        assert_eq!(fs::read_to_string(&script).unwrap(), "return 1");
        assert_eq!(task.message_queue.cursor(), 0);
        assert_eq!(
            task.tree
                .lock()
                .unwrap()
                .get_instance(id)
                .unwrap()
                .properties()[&rbx_dom_weak::ustr("Source")],
            Variant::String("return 1".into())
        );
        task.handle_tree_event(source_update(id, "return 3"));
        assert_eq!(fs::read_to_string(&script).unwrap(), "return 3");
        assert_eq!(task.message_queue.cursor(), 1);
    }

    #[test]
    fn source_write_uses_the_init_file_of_a_project_node() {
        let task = fixture();
        task.vfs
            .write("/project/src/nested/init.luau", "return 1")
            .unwrap();
        task.vfs.write("/project/default.project.json", r#"{"name":"Root","tree":{"$className":"Folder","Scripts":{"$path":"src/nested"}}}"#).unwrap();
        let snapshot = snapshot_from_vfs(
            &InstanceContext::default(),
            &task.vfs,
            "/project/default.project.json".as_ref(),
        )
        .unwrap()
        .unwrap();
        *task.tree.lock().unwrap() = RojoTree::new(snapshot);
        let id = task
            .tree
            .lock()
            .unwrap()
            .get_ids_at_path("/project/src/nested".as_ref())[0];
        task.handle_tree_event(source_update(id, "return 3"));
        assert_eq!(
            task.vfs
                .read_to_string("/project/src/nested/init.luau")
                .unwrap()
                .as_str(),
            "return 3"
        );
        assert_eq!(task.message_queue.cursor(), 1);
    }

    #[test]
    fn generated_json_source_is_not_overwritten_with_luau() {
        let task = fixture();
        let path = "/project/src/data.json";
        let contents = r#"{"answer":42}"#;
        task.vfs.write(path, contents).unwrap();
        task.handle_vfs_event(VfsEvent::Create(path.into()));
        let id = task.tree.lock().unwrap().get_ids_at_path(path.as_ref())[0];
        let cursor = task.message_queue.cursor();
        task.handle_tree_event(source_update(id, "return 3"));
        assert_eq!(task.vfs.read_to_string(path).unwrap().as_str(), contents);
        assert_eq!(task.message_queue.cursor(), cursor);
    }

    #[test]
    fn unchanged_directory_event_does_not_publish_a_reference_patch() {
        let task = fixture();
        task.vfs
            .write(
                "/project/src/pointer.model.json",
                r#"{"className":"ObjectValue","attributes":{"Rojo_Target_Value":"target"}}"#,
            )
            .unwrap();
        task.vfs
            .write(
                "/project/src/target.model.json",
                r#"{"className":"ObjectValue","attributes":{"Rojo_Id":"target"}}"#,
            )
            .unwrap();
        let snapshot = snapshot_from_vfs(
            &InstanceContext::default(),
            &task.vfs,
            "/project/src".as_ref(),
        )
        .unwrap();
        let mut tree = RojoTree::new(crate::snapshot::InstanceSnapshot::new());
        let root = tree.get_root_id();
        let patch = compute_patch_set(snapshot, &tree, root);
        apply_patch_set(&mut tree, patch);
        *task.tree.lock().unwrap() = tree;
        task.handle_vfs_event(VfsEvent::Write("/project/src".into()));
        if task.message_queue.cursor() != 0 {
            panic!(
                "unexpected patch: {:?}",
                futures::executor::block_on(task.message_queue.subscribe(0)).unwrap()
            );
        }
    }

    #[test]
    fn text_module_source_updates_decode_before_writing_and_reject_without_publishing() {
        for explicit in [false, true] {
            let tree = if explicit {
                r#"{"$className":"Folder","Items":{"$path":"items.csv"}}"#
            } else {
                r#"{"$className":"Folder","$path":"source"}"#
            };
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir(dir.path().join("source")).unwrap();
            let path = dir.path().join(if explicit {
                "items.csv"
            } else {
                "source/items.csv"
            });
            fs::write(&path, "old").unwrap();
            let project = dir.path().join("default.project.json");
            fs::write(&project, format!(r#"{{"name":"Root","syncRules":[{{"pattern":"**/*.csv","use":"textModule","suffix":".csv"}}],"tree":{tree}}}"#)).unwrap();
            let vfs = Arc::new(Vfs::new_default().unwrap());
            vfs.set_watch_enabled(false);
            let snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, &project)
                .unwrap()
                .unwrap();
            let task = JobThreadContext {
                tree: Arc::new(Mutex::new(RojoTree::new(snapshot))),
                vfs,
                message_queue: Arc::new(MessageQueue::new()),
                frozen_project: None,
            };
            let id = task
                .tree
                .lock()
                .unwrap()
                .get_ids_at_path(&task.vfs.canonicalize(&path).unwrap())[0];
            let initial = task
                .tree
                .lock()
                .unwrap()
                .get_instance(id)
                .unwrap()
                .properties()[&"Source".into()]
                .clone();
            let cursor = task.message_queue.cursor();
            for source in [
                "return os.time()",
                "return 'not canonical'\n",
                "return \"\\255\"\n",
            ] {
                task.handle_tree_event(source_update(id, source));
                assert_eq!(task.vfs.read_to_string(&path).unwrap().as_str(), "old");
                assert_eq!(task.message_queue.cursor(), cursor);
                assert_eq!(
                    task.tree
                        .lock()
                        .unwrap()
                        .get_instance(id)
                        .unwrap()
                        .properties()[&"Source".into()],
                    initial
                );
            }
            let source = "return \"new\\000123\\013\\010ä\"\n";
            task.handle_tree_event(source_update(id, source));
            assert_eq!(
                task.vfs.read_to_string(&path).unwrap().as_str(),
                "new\x00123\r\nä"
            );
            assert!(task.message_queue.cursor() > cursor);
            assert_eq!(
                task.tree
                    .lock()
                    .unwrap()
                    .get_instance(id)
                    .unwrap()
                    .properties()[&"Source".into()],
                Variant::String(source.into())
            );
        }
    }

    fn fixture() -> JobThreadContext {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot(
            "/project",
            VfsSnapshot::dir([(
                "src",
                VfsSnapshot::dir([(
                    "nested",
                    VfsSnapshot::dir([("child.luau", VfsSnapshot::file("return 1"))]),
                )]),
            )]),
        )
        .unwrap();
        let vfs = Arc::new(Vfs::new(fs));
        let snapshot =
            snapshot_from_vfs(&InstanceContext::default(), &vfs, "/project/src".as_ref())
                .unwrap()
                .unwrap();
        JobThreadContext {
            tree: Arc::new(Mutex::new(RojoTree::new(snapshot))),
            vfs,
            message_queue: Arc::new(MessageQueue::new()),
            frozen_project: None,
        }
    }

    #[test]
    fn deleted_create_and_write_targets_do_not_stop_later_updates() {
        let task = fixture();
        for event in [
            VfsEvent::Create("/project/src/missing.luau".into()),
            VfsEvent::Write("/project/src/missing.luau".into()),
        ] {
            task.handle_vfs_event(event);
        }
        task.vfs
            .write("/project/src/canary.luau", "return true")
            .unwrap();
        task.handle_vfs_event(VfsEvent::Create("/project/src/canary.luau".into()));
        let tree = task.tree.lock().unwrap();
        assert_eq!(
            tree.get_ids_at_path("/project/src/canary.luau".as_ref())
                .len(),
            1
        );
    }

    #[test]
    fn removing_children_after_their_parent_has_gone_cleans_metadata() {
        let task = fixture();
        task.vfs.remove_dir_all("/project/src/nested").unwrap();
        for path in ["/project/src/nested/child.luau", "/project/src/nested"] {
            task.handle_vfs_event(VfsEvent::Remove(path.into()));
        }
        let tree = task.tree.lock().unwrap();
        assert!(tree
            .get_ids_at_path("/project/src/nested/child.luau".as_ref())
            .is_empty());
        assert!(tree
            .get_ids_at_path("/project/src/nested".as_ref())
            .is_empty());
        assert!(tree.root().children().is_empty());
    }

    #[test]
    fn delayed_remove_after_file_becomes_directory_uses_current_state() {
        let task = fixture();
        task.vfs
            .remove_file("/project/src/nested/child.luau")
            .unwrap();
        task.vfs
            .create_dir("/project/src/nested/child.luau")
            .unwrap();
        task.vfs
            .write("/project/src/nested/child.luau/init.luau", "return 2")
            .unwrap();
        task.handle_vfs_event(VfsEvent::Remove("/project/src/nested/child.luau".into()));
        let tree = task.tree.lock().unwrap();
        let ids = tree.get_ids_at_path("/project/src/nested/child.luau".as_ref());
        assert_eq!(ids.len(), 1);
        assert_eq!(
            tree.get_instance(ids[0])
                .unwrap()
                .properties()
                .get(&rbx_dom_weak::ustr("Source")),
            Some(&Variant::String("return 2".into()))
        );
    }

    #[test]
    fn deleting_the_root_source_preserves_the_tree_until_recreated() {
        let task = fixture();
        task.vfs.remove_dir_all("/project/src").unwrap();
        task.handle_vfs_event(VfsEvent::Remove("/project/src".into()));
        task.vfs.create_dir("/project/src").unwrap();
        task.vfs
            .write("/project/src/canary.luau", "return true")
            .unwrap();
        task.handle_vfs_event(VfsEvent::Create("/project/src".into()));
        let tree = task.tree.lock().unwrap();
        assert_eq!(
            tree.get_ids_at_path("/project/src/canary.luau".as_ref())
                .len(),
            1
        );
        assert!(tree
            .get_ids_at_path("/project/src/nested".as_ref())
            .is_empty());
    }

    #[test]
    fn optional_project_root_disappearing_keeps_its_identity() {
        let task = fixture();
        task.vfs
            .write(
                "/project/default.project.json",
                r#"{"name":"Root","tree":{"$path":{"optional":"src"}}}"#,
            )
            .unwrap();
        let snapshot = snapshot_from_vfs(
            &InstanceContext::default(),
            &task.vfs,
            "/project/default.project.json".as_ref(),
        )
        .unwrap()
        .unwrap();
        *task.tree.lock().unwrap() = RojoTree::new(snapshot);
        let root = task.tree.lock().unwrap().get_root_id();
        task.vfs.remove_dir_all("/project/src").unwrap();
        task.handle_vfs_event(VfsEvent::Remove("/project/src".into()));
        assert_eq!(task.tree.lock().unwrap().get_root_id(), root);
        task.vfs.create_dir("/project/src").unwrap();
        task.vfs
            .write("/project/src/canary.luau", "return true")
            .unwrap();
        task.handle_vfs_event(VfsEvent::Create("/project/src".into()));
        assert_eq!(
            task.tree
                .lock()
                .unwrap()
                .get_ids_at_path("/project/src/canary.luau".as_ref())
                .len(),
            1
        );
    }
}
