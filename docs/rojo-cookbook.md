# Rojo cookbook for file and AI-assisted workflows

Examples target this fork's actual CLI/project format. Begin with the
[runnable workflow](fully-managed-workflow.md) and use its `$Rojo`, `$Project`
and `$Out` variables in PowerShell7.

## Add code without changing the hierarchy accidentally

| File/layout | Meaning |
|---|---|
| `Thing.luau` / `Thing.lua` | ModuleScript |
| `Boot.server.luau` | Server script; V2 uses Script + server RunContext |
| `Boot.client.luau` | Client script; V2 uses Script + client RunContext |
| `Feature/init.luau` plus siblings | Feature becomes a ModuleScript containing those siblings |
| `$path: "Feature/init.luau"` | Explicit single-file binding; does not import its sibling files |
| `Feature/init.meta.json` | Metadata for that filesystem Feature instance |
| `Thing.meta.json` beside `Thing.luau` | Metadata for Thing, not another child |

Don't map both a directory and its init source unintentionally. Duplicate paths
are separate bindings, not shared identity. Keep typed StarterPlayer declarations
in the project tree, not metadata; see the tutorial's container section.

## Put properties in the right representation

Adjacent metadata for Greeting:

```json
{
  "properties": { "Archivable": true },
  "attributes": { "Owner": "shared" }
}
```

The equivalent inline project node uses `$properties` and `$attributes`.
Metadata uses `properties` and `attributes`; it does not accept project children
or `$path` mappings. Use a `.model.json` for an actual model tree. The runnable
Demo shows canonical `className`, `properties`, `attributes` and `children`.
Reference attributes such as `Rojo_Target_PrimaryPart: "demo-pad"` point to the
target's `Rojo_Id`/project `$id`, not a transient wire UUID.

For scaling, retain the stored factor together with baked geometry; see
[Model.Scale](model-scale.md). Don't apply a saved factor a second time on import.

## Share defaults without copying metadata to every file

Project-level example:

```json
{
  "syncRules": [
    {
      "pattern": "**/*.luau",
      "defaults": {
        "tags": ["Shared"],
        "attributes": { "Owner": "tools" }
      }
    }
  ]
}
```

This fragment belongs beside the existing `tree`; it is not a full project.
A defaults-only rule doesn't replace normal suffix routing. Explicit metadata
or project values win per key. To remove one inherited default attribute, use
adjacent metadata `{"ruleDefaults":{"disabledAttributes":["Owner"]}}`.
Keep genuinely shared values shared when reviewing syncback output.

## Keep scripts in files while artists own Part properties

Add this field to the project:

```json
{
  "syncAuthority": [
    { "className": "LuaSourceContainer", "properties": "filesystem" },
    { "className": "BasePart", "properties": "studio" }
  ]
}
```

Exact classes win, otherwise the nearest configured superclass wins. Structure
defaults to filesystem. Nested omission inherits; `syncAuthority: []` restores
legacy behavior. This is property authority per instance, not an excluded whole
subtree. Matching Studio fields survive initial/live file sync; script Source
follows files. Explicit syncback exports Studio fields while retaining filesystem
fields. Existing native parent bytes can remain unchanged while Studio-owned
descendants export. General live `twoWay` properties/structure remain disabled.

Authority changes, destructive replacement and unsafe recovery stop before
discarding Studio values and require reconnect review. Do not auto-accept those
prompts. Filesystem Model.Scale over Studio-owned descendants is refused because
the native operation would mutate their geometry indirectly. Full contract:
[class authority](class-authority.md).

## Exclude a whole class from syncback

`"syncbackRules": {"ignoreClasses":["Camera"]}` retains existing excluded disk
subtrees, including descendants, and omits new excluded input. It is stronger
than per-instance property selection. Ambiguous or unsafe reference preservation
refuses before writes. This does not change live sync ownership.

## Preview imports and preserve an existing place

```powershell
& $Rojo syncback $Project -i "$Out\edited.rbxl" --dry-run --list
& $Rojo build $Project --input "$Out\original.rbxl" -o "$Out\updated.rbxl"
```

The first command writes nothing. The second overlays known project data onto a
separate existing place input; review ambiguity/deletion errors rather than
silently dropping unknown content. `syncback --watch` additionally requires `-y`
or `--dry-run`; choose deliberately before watching saved place files.

## Diagnose a failed or surprising sync

1. Read the serve terminal, resolved project and emitted sourcemap paths.
2. Check CLI/plugin version/capabilities and place restrictions. A malformed
   handshake now reports its URL instead of indexing a number.
3. Inspect the failed patch. A missing parent keeps its child subtree unapplied;
   repair the original parent failure, then retry/reconnect.
4. Keep dirty Source/Studio-owned values before reconnect review. Name-only
   guesses are not safe adoption. Opt-in adoption verifies readable fields;
   opaque serialized values stay explicitly unverified.
5. Rebuild after explicit syncback and inspect the resulting diff.

Use `serve --sync-error-log --sync-warn-log` for mapped Studio diagnostics.
Existing `--debugger-plans` supports automatic combined-Run plans after
native consent; held pauses/separate Play are unsupported. Neither the CLI nor
this cookbook grants an agent permission to publish or start another play mode.
