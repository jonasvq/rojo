use std::{path::Path, str};

use anyhow::{ensure, Context as _};
use indexmap::IndexMap;
use memofs::Vfs;
use rbx_dom_weak::{
    types::{Enum, Variant},
    ustr, HashMapExt as _, Ustr, UstrMap,
};

use crate::{
    resolution::UnresolvedValue,
    snapshot::{InstanceContext, InstanceMetadata, InstanceSnapshot},
    syncback::{FsSnapshot, SyncbackReturn, SyncbackSnapshot},
};

use super::{
    dir::{snapshot_dir_no_meta, syncback_dir_no_meta},
    meta_file::{AdjacentMetadata, DirectoryMetadata},
    Middleware, PathExt as _,
};

#[derive(Debug)]
pub enum ScriptType {
    Server,
    Client,
    Module,
    Plugin,
    LegacyServer,
    LegacyClient,
    RunContextServer,
    RunContextClient,
}

fn script_defaults(
    script_type: ScriptType,
    emit_legacy_scripts: bool,
) -> (&'static str, Option<u32>) {
    let run_context_enums = &rbx_reflection_database::get()
        .unwrap()
        .enums
        .get("RunContext")
        .expect("Unable to get RunContext enums!")
        .items;

    let (class_name, run_context) = match script_type {
        ScriptType::Server => {
            if emit_legacy_scripts {
                ("Script", run_context_enums.get("Legacy"))
            } else {
                ("Script", run_context_enums.get("Server"))
            }
        }
        ScriptType::Client => {
            if emit_legacy_scripts {
                ("LocalScript", None)
            } else {
                ("Script", run_context_enums.get("Client"))
            }
        }
        ScriptType::Module => ("ModuleScript", None),
        ScriptType::Plugin => ("Script", run_context_enums.get("Plugin")),
        ScriptType::LegacyServer => ("Script", run_context_enums.get("Legacy")),
        ScriptType::LegacyClient => ("LocalScript", None),
        ScriptType::RunContextServer => ("Script", run_context_enums.get("Server")),
        ScriptType::RunContextClient => ("Script", run_context_enums.get("Client")),
    };

    (class_name, run_context.copied())
}

pub(super) fn destination_context(snapshot: &SyncbackSnapshot) -> anyhow::Result<InstanceContext> {
    if let Some(old) = snapshot.old_inst() {
        return Ok(old.metadata().context.clone());
    }
    fn names(dom: &rbx_dom_weak::WeakDom, mut id: rbx_dom_weak::types::Ref) -> Vec<&str> {
        let mut names = Vec::new();
        while let Some(instance) = dom.get_by_ref(id) {
            if id == dom.root_ref() {
                break;
            }
            names.push(instance.name.as_str());
            id = instance.parent();
        }
        names.reverse();
        names
    }
    let parent_names = names(snapshot.new_tree(), snapshot.new_inst().parent());
    let mut ancestor = snapshot.path.parent();
    while let Some(path) = ancestor {
        // Aliased physical directories can belong to different project contexts.
        // Match the actual instance ancestry before considering those contexts.
        let ids: Vec<_> = snapshot
            .old_rojo_tree()
            .get_ids_at_path(path)
            .iter()
            .copied()
            .filter(|id| parent_names.starts_with(&names(snapshot.old_tree(), *id)))
            .collect();
        if let Some(first) = ids.first() {
            let context = &snapshot
                .get_old_instance(*first)
                .unwrap()
                .metadata()
                .context;
            ensure!(
                ids.iter().all(
                    |id| snapshot.get_old_instance(*id).unwrap().metadata().context == *context
                ),
                "Ambiguous script destination context: {}",
                path.display()
            );
            return Ok(context.clone());
        }
        ancestor = path.parent();
    }
    let mut context = InstanceContext::with_emit_legacy_scripts(Some(
        snapshot.project().effective_emit_legacy_scripts(),
    ));
    context.add_sync_rules(snapshot.project().sync_rules.iter().map(|rule| {
        crate::snapshot::SyncRule {
            base_path: snapshot.project().folder_location().to_owned(),
            ..rule.clone()
        }
    }));
    Ok(context)
}

fn destination_defaults(
    snapshot: &SyncbackSnapshot,
    path: &Path,
) -> anyhow::Result<(InstanceContext, ScriptType, String)> {
    let context = destination_context(snapshot)?;
    let rule = context
        .get_user_sync_rule(path)
        .or_else(|| {
            super::default_sync_rules()
                .iter()
                .find(|rule| rule.matches(path))
        })
        .with_context(|| format!("No native script rule for {}", path.display()))?;
    let script_type = rule
        .middleware
        .and_then(script_type_for_middleware)
        .with_context(|| {
            format!(
                "Native script destination {} is bound to incompatible middleware {:?}",
                path.display(),
                rule.middleware
            )
        })?;
    let name = rule.file_name_for_path(path)?.to_owned();
    Ok((context, script_type, name))
}

fn script_type_for_middleware(middleware: Middleware) -> Option<ScriptType> {
    Some(match middleware {
        Middleware::ServerScript => ScriptType::Server,
        Middleware::ClientScript => ScriptType::Client,
        Middleware::ModuleScript => ScriptType::Module,
        Middleware::PluginScript => ScriptType::Plugin,
        Middleware::LegacyServerScript => ScriptType::LegacyServer,
        Middleware::LegacyClientScript => ScriptType::LegacyClient,
        Middleware::RunContextServerScript => ScriptType::RunContextServer,
        Middleware::RunContextClientScript => ScriptType::RunContextClient,
        Middleware::ServerScriptDir => ScriptType::Server,
        Middleware::ClientScriptDir => ScriptType::Client,
        Middleware::ModuleScriptDir => ScriptType::Module,
        Middleware::PluginScriptDir => ScriptType::Plugin,
        _ => return None,
    })
}

pub(super) fn project_run_context(snapshot: &SyncbackSnapshot) -> anyhow::Result<Option<Variant>> {
    if snapshot.new_inst().class != "Script" {
        return Ok(None);
    }
    let middleware = snapshot
        .middleware
        .context("Project source middleware missing")?;
    let (context, script_type) = if middleware.is_dir() {
        let (middleware, _, _) = super::get_dir_middleware(snapshot.vfs(), &snapshot.path)?;
        (
            destination_context(snapshot)?,
            script_type_for_middleware(middleware)
                .context("Project script directory has incompatible middleware")?,
        )
    } else {
        let (context, script_type, _) = destination_defaults(snapshot, &snapshot.path)?;
        (context, script_type)
    };
    let (_, run_context) = script_representation(snapshot, script_type, &context)?;
    Ok(run_context.map(|value| Enum::from_u32(value).into()))
}

fn script_context(properties: &UstrMap<Variant>) -> anyhow::Result<u32> {
    let value = properties.get(&ustr("RunContext")).or_else(|| {
        rbx_reflection_database::get()
            .ok()?
            .classes
            .get("Script")?
            .default_properties
            .get("RunContext")
    });
    match value {
        Some(Variant::Enum(value)) => Ok(value.to_u32()),
        _ => anyhow::bail!("Script RunContext must be an enum"),
    }
}

fn script_representation(
    snapshot: &SyncbackSnapshot,
    script_type: ScriptType,
    context: &InstanceContext,
) -> anyhow::Result<(Option<Ustr>, Option<u32>)> {
    let instance = snapshot.new_inst();
    let (base_class, mut run_context) = script_defaults(script_type, context.emit_legacy_scripts);
    let state = super::rule_defaults::syncback_state(snapshot)?;
    let mut class = Ustr::from(base_class);
    if let Some(state) = &state {
        if !state.mask.disable_class_name {
            class = state.class_name.unwrap_or(class);
        }
        if !state.mask.disabled_properties.contains(&ustr("RunContext")) {
            if let Some(Variant::Enum(value)) = state.properties.get(&ustr("RunContext")) {
                run_context = Some(value.to_u32());
            }
        }
    }
    ensure!(
        class == instance.class
            || (matches!(class.as_str(), "Script" | "LocalScript")
                && matches!(instance.class.as_str(), "Script" | "LocalScript")),
        "Incompatible native script binding at {}: {class} cannot represent {}",
        snapshot.path.display(),
        instance.class
    );
    let class_override = (class != instance.class
        || state.as_ref().is_some_and(|s| s.explicit_class))
    .then_some(instance.class);
    if instance.class == "LocalScript" {
        ensure!(
            !instance.properties.contains_key(&ustr("RunContext")),
            "LocalScript cannot have RunContext: {}",
            snapshot.path.display()
        );
    }
    let forced_context = if instance.class == "Script" {
        let actual = script_context(&instance.properties)?;
        // A LocalScript-to-Script class override uses the engine default.
        let expected = run_context.unwrap_or(script_context(&UstrMap::new())?);
        (actual != expected
            || state
                .as_ref()
                .is_some_and(|s| s.explicit_properties.contains(&ustr("RunContext"))))
        .then_some(actual)
    } else {
        None
    };
    Ok((class_override, forced_context))
}

fn preserve_run_context(properties: &mut IndexMap<Ustr, UnresolvedValue>, value: Option<u32>) {
    if let Some(value) = value {
        properties.insert(
            ustr("RunContext"),
            UnresolvedValue::from_variant_unambiguous(Enum::from_u32(value).into()),
        );
        properties.sort_by(|a, _, b, _| a.as_str().cmp(b.as_str()));
    }
}

pub(super) fn metadata_destination(vfs: &Vfs, path: &Path) -> anyhow::Result<std::path::PathBuf> {
    let jsonc = path.with_extension("jsonc");
    let has_jsonc = vfs.exists(&jsonc)?;
    ensure!(
        !has_jsonc || !vfs.exists(path)?,
        "Native script has both JSON and JSONC metadata; resolve the duplicate before syncback: {}",
        path.display()
    );
    Ok(if has_jsonc { jsonc } else { path.to_owned() })
}

/// Core routine for turning Lua files into snapshots.
pub fn snapshot_lua(
    context: &InstanceContext,
    vfs: &Vfs,
    path: &Path,
    name: &str,
    script_type: ScriptType,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let (class_name, run_context) = script_defaults(script_type, context.emit_legacy_scripts);

    let contents = vfs.read_to_string_lf_normalized(path)?;
    let contents_str = contents.as_str();

    let mut properties = UstrMap::with_capacity(2);
    properties.insert(ustr("Source"), contents_str.into());

    if let Some(run_context) = run_context {
        properties.insert(ustr("RunContext"), Enum::from_u32(run_context).into());
    }

    let mut snapshot = InstanceSnapshot::new()
        .name(name)
        .class_name(class_name)
        .properties(properties)
        .metadata(
            InstanceMetadata::new()
                .instigating_source(path)
                .relevant_paths(vec![vfs.canonicalize(path)?])
                .context(context),
        );

    AdjacentMetadata::read_and_apply_all(vfs, path, name, &mut snapshot)?;

    Ok(Some(snapshot))
}

/// Attempts to snapshot an 'init' Lua script contained inside of a folder with
/// the given name.
///
/// Scripts named `init.lua`, `init.server.lua`, or `init.client.lua` usurp
/// their parents, which acts similarly to `__init__.py` from the Python world.
pub fn snapshot_lua_init(
    context: &InstanceContext,
    vfs: &Vfs,
    init_path: &Path,
    name: &str,
    script_type: ScriptType,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let folder_path = init_path.parent().unwrap();
    let dir_snapshot = snapshot_dir_no_meta(context, vfs, folder_path, name)?.unwrap();

    if dir_snapshot.class_name != "Folder" {
        anyhow::bail!(
            "init.lua, init.server.lua, and init.client.lua can \
             only be used if the instance produced by the containing \
             directory would be a Folder.\n\
             \n\
             The directory {} turned into an instance of class {}.",
            folder_path.display(),
            dir_snapshot.class_name
        );
    }

    let mut init_snapshot =
        snapshot_lua(context, vfs, init_path, &dir_snapshot.name, script_type)?.unwrap();

    init_snapshot.children = dir_snapshot.children;
    let prepared_defaults = init_snapshot.metadata.rule_defaults.take();
    init_snapshot.metadata = dir_snapshot.metadata;
    init_snapshot.metadata.rule_defaults = prepared_defaults;
    // The directory snapshot middleware includes all possible init paths
    // so we don't need to add it here.

    DirectoryMetadata::read_and_apply_all(vfs, folder_path, &mut init_snapshot)?;

    Ok(Some(init_snapshot))
}

pub fn syncback_lua<'sync>(
    snapshot: &SyncbackSnapshot<'sync>,
) -> anyhow::Result<SyncbackReturn<'sync>> {
    let new_inst = snapshot.new_inst();
    ensure!(new_inst.children().is_empty(),
        "Cannot add children to native script file {}; an explicit source-layout migration is required", snapshot.path.display());
    let (context, script_type, meta_name) = destination_defaults(snapshot, &snapshot.path)?;
    let project_bound = snapshot.old_inst().is_some_and(|old| {
        old.metadata()
            .instigating_source
            .as_ref()
            .is_some_and(|source| source.path() != snapshot.path)
    });
    ensure!(
        project_bound || meta_name == new_inst.name,
        "Destination rule would rename native script {} to {meta_name}: {}",
        new_inst.name,
        snapshot.path.display()
    );
    let representation = script_representation(snapshot, script_type, &context)?;

    let contents = if let Some(Variant::String(source)) = new_inst.properties.get(&ustr("Source")) {
        source.as_bytes().to_vec()
    } else {
        anyhow::bail!("Scripts must have a `Source` property that is a String")
    };
    let mut fs_snapshot = FsSnapshot::new();
    fs_snapshot.add_file(&snapshot.path, contents);

    let meta = AdjacentMetadata::from_syncback_snapshot(snapshot, snapshot.path.clone())?;
    if let Some(mut meta) = meta {
        meta.id = snapshot.old_inst().and_then(|old| {
            old.metadata()
                .specified_id
                .as_ref()
                .map(ToString::to_string)
        });
        meta.class_name = representation.0;
        preserve_run_context(&mut meta.properties, representation.1);
        // Scripts have relatively few properties that we care about, so shifting
        // is fine.
        meta.properties.shift_remove(&ustr("Source"));

        let parent_location = snapshot.path.parent_err()?;
        let meta_path = metadata_destination(
            snapshot.vfs(),
            &parent_location.join(format!("{meta_name}.meta.json")),
        )?;
        if !meta.is_empty() || snapshot.vfs().exists(&meta_path)? {
            fs_snapshot.add_file(
                meta_path,
                serde_json::to_vec_pretty(&meta).context("cannot serialize metadata")?,
            );
        }
    }

    Ok(SyncbackReturn {
        fs_snapshot,
        // Scripts don't have a child!
        children: Vec::new(),
        removed_children: Vec::new(),
    })
}

pub fn syncback_lua_init<'sync>(
    script_type: ScriptType,
    snapshot: &SyncbackSnapshot<'sync>,
) -> anyhow::Result<SyncbackReturn<'sync>> {
    let new_inst = snapshot.new_inst();
    if let Some(old) = snapshot.old_inst() {
        ensure!(old.name() == new_inst.name,
            "Renaming an existing native script directory requires an explicit source-layout migration: {}", snapshot.path.display());
    }
    let default_path = snapshot.path.join(match script_type {
        ScriptType::Server => "init.server.luau",
        ScriptType::Client => "init.client.luau",
        ScriptType::Module => "init.luau",
        ScriptType::Plugin => "init.plugin.luau",
        _ => anyhow::bail!("syncback is not yet implemented for {script_type:?}"),
    });
    let path = if snapshot.old.is_some() {
        let (middleware, _, path) = super::get_dir_middleware(snapshot.vfs(), &snapshot.path)?;
        ensure!(
            matches!(
                middleware,
                Middleware::ServerScriptDir
                    | Middleware::ClientScriptDir
                    | Middleware::ModuleScriptDir
                    | Middleware::PluginScriptDir
            ),
            "Native script directory changed representation: {}",
            snapshot.path.display()
        );
        path
    } else {
        default_path
    };
    let context = destination_context(snapshot)?;
    let representation = script_representation(snapshot, script_type, &context)?;

    let contents = if let Some(Variant::String(source)) = new_inst.properties.get(&ustr("Source")) {
        source.as_bytes().to_vec()
    } else {
        anyhow::bail!("Scripts must have a `Source` property that is a String")
    };

    let mut dir_syncback = syncback_dir_no_meta(snapshot)?;
    dir_syncback.fs_snapshot.add_file(&path, contents);

    let meta = DirectoryMetadata::from_syncback_snapshot(snapshot, snapshot.path.clone())?;
    if let Some(mut meta) = meta {
        meta.id = snapshot.old_inst().and_then(|old| {
            old.metadata()
                .specified_id
                .as_ref()
                .map(ToString::to_string)
        });
        meta.class_name = representation.0;
        preserve_run_context(&mut meta.properties, representation.1);
        // Scripts have relatively few properties that we care about, so shifting
        // is fine.
        meta.properties.shift_remove(&ustr("Source"));

        let meta_path =
            metadata_destination(snapshot.vfs(), &snapshot.path.join("init.meta.json"))?;
        if !meta.is_empty() || snapshot.vfs().exists(&meta_path)? {
            dir_syncback.fs_snapshot.add_file(
                meta_path,
                serde_json::to_vec_pretty(&meta)
                    .context("could not serialize new init.meta.json")?,
            );
        }
    }

    Ok(dir_syncback)
}

#[cfg(test)]
mod test {
    use super::*;

    use memofs::{InMemoryFs, VfsSnapshot};

    #[test]
    fn class_module_from_vfs() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/foo.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(true)),
            &vfs,
            Path::new("/foo.lua"),
            "foo",
            ScriptType::Module,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn runcontext_module_from_vfs() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/foo.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(false)),
            &vfs,
            Path::new("/foo.lua"),
            "foo",
            ScriptType::Module,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn plugin_module_from_vfs() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/foo.plugin.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(false)),
            &vfs,
            Path::new("/foo.plugin.lua"),
            "foo",
            ScriptType::Plugin,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn class_server_from_vfs() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/foo.server.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(true)),
            &vfs,
            Path::new("/foo.server.lua"),
            "foo",
            ScriptType::Server,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn runcontext_server_from_vfs() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/foo.server.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(false)),
            &vfs,
            Path::new("/foo.server.lua"),
            "foo",
            ScriptType::Server,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn class_client_from_vfs() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/foo.client.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(true)),
            &vfs,
            Path::new("/foo.client.lua"),
            "foo",
            ScriptType::Client,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn runcontext_client_from_vfs() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/foo.client.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(false)),
            &vfs,
            Path::new("/foo.client.lua"),
            "foo",
            ScriptType::Client,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn init_module_from_vfs() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/root",
            VfsSnapshot::dir([("init.lua", VfsSnapshot::file("Hello!"))]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua_init(
            &InstanceContext::with_emit_legacy_scripts(Some(true)),
            &vfs,
            Path::new("/root/init.lua"),
            "root",
            ScriptType::Module,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn init_module_from_vfs_with_meta() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/root",
            VfsSnapshot::dir([
                ("init.lua", VfsSnapshot::file("Hello!")),
                (
                    "init.meta.json",
                    VfsSnapshot::file(r#"{"id": "manually specified"}"#),
                ),
            ]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua_init(
            &InstanceContext::with_emit_legacy_scripts(Some(true)),
            &vfs,
            Path::new("/root/init.lua"),
            "root",
            ScriptType::Module,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn class_module_with_meta() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/foo.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();
        imfs.load_snapshot(
            "/foo.meta.json",
            VfsSnapshot::file(
                r#"
                    {
                        "ignoreUnknownInstances": true
                    }
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(true)),
            &vfs,
            Path::new("/foo.lua"),
            "foo",
            ScriptType::Module,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn runcontext_module_with_meta() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/foo.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();
        imfs.load_snapshot(
            "/foo.meta.json",
            VfsSnapshot::file(
                r#"
                    {
                        "ignoreUnknownInstances": true
                    }
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(false)),
            &vfs,
            Path::new("/foo.lua"),
            "foo",
            ScriptType::Module,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn class_script_with_meta() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/foo.server.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();
        imfs.load_snapshot(
            "/foo.meta.json",
            VfsSnapshot::file(
                r#"
                    {
                        "ignoreUnknownInstances": true
                    }
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(true)),
            &vfs,
            Path::new("/foo.server.lua"),
            "foo",
            ScriptType::Server,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn runcontext_script_with_meta() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/foo.server.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();
        imfs.load_snapshot(
            "/foo.meta.json",
            VfsSnapshot::file(
                r#"
                    {
                        "ignoreUnknownInstances": true
                    }
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(false)),
            &vfs,
            Path::new("/foo.server.lua"),
            "foo",
            ScriptType::Server,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn class_script_disabled() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/bar.server.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();
        imfs.load_snapshot(
            "/bar.meta.json",
            VfsSnapshot::file(
                r#"
                    {
                        "properties": {
                            "Disabled": true
                        }
                    }
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(true)),
            &vfs,
            Path::new("/bar.server.lua"),
            "bar",
            ScriptType::Server,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn runcontext_script_disabled() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/bar.server.lua", VfsSnapshot::file("Hello there!"))
            .unwrap();
        imfs.load_snapshot(
            "/bar.meta.json",
            VfsSnapshot::file(
                r#"
                    {
                        "properties": {
                            "Disabled": true
                        }
                    }
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_lua(
            &InstanceContext::with_emit_legacy_scripts(Some(false)),
            &vfs,
            Path::new("/bar.server.lua"),
            "bar",
            ScriptType::Server,
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }
}
