# Rojo Changelog

<!-- DIRECTIONS
Thanks for contributing to Rojo! Please add your change to the "Unreleased" section below.

Use the following format:

```
## Unreleased

* Preserve a model root's snapshot identity through `$path` project layers so self references and child references resolve to the final managed root, including nested projects, aliased model sources and reloads. ([#1292])

* Add `syncbackRules.ignoreClasses` for classes and derived classes: retain their existing disk subtrees and required ancestors, omit new excluded input, and preserve embedded model values and reference identities. Reject ambiguous or unsafe reference preservation before writing files. ([#1299])

* Report circular project references with their canonical path chain instead of overflowing the stack, while allowing independent aliases and later retries. ([#644])
* Allow explicit `$path` bindings to init source files without importing sibling files, and avoid duplicate case-variant init children on Windows. ([#906], [#1016], [#1283])
* Accept one leading UTF-8 byte order mark in JSON/JSONC projects, models, modules and metadata while preserving string contents and contextual parse errors. ([#1307])
* Remove large instance batches with one stable sibling-list filter per parent, preserving metadata, references and surviving instance order through the local DOM fork. ([#1136])
* Match large groups of snapshot children through ordered name/class candidate lists, avoiding repeated sibling scans while preserving duplicate and script-source identity rules. ([#1136])
* Sync immutable MeshPart mesh content through Roblox's mesh-loading API while preserving instance identity, children, transforms and texture; support live changes and removal for both `MeshContent` and legacy `MeshId`. ([#1221])
* Preserve Studio's active, unmanaged Workspace camera and viewpoint when connecting a fully managed project; explicitly managed cameras still follow project updates and deletions. ([#981])
* Made a change to Rojo ([#0000])

[#0000]:  https://github.com/rojo-rbx/rojo/pull/0000
```

When adding your link definition, please keep them in ascending order (lowest number first).

Making a new release? Simply add the new header with the version and date underneath "Unreleased" and add the version link, like so:

```
## Unreleased

## [0.0.0] (January 1st, 1970)

* ... the changes here

[0.0.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.0.0
[#0000]: ... the other links go after the release link
```
-->

## Unreleased

## [8.0.0] (October 26th, 2026)

Independent jonasvq fork based on upstream7.7.1; CLI and Studio plugin must both come from this fork. Release assets include guides/examples and checksums.

* Add a runnable fully managed tutorial, cookbook and coding-agent guide, with a checked-in example, CLI roundtrips and bounded native Edit verification. ([#812], [#166])

* Validate server handshakes before reading protocol, session or capability fields, even when optional typechecking is disabled. Report malformed or undecodable replies with the server URL and connection guidance instead of a scalar-indexing exception. ([#1302])

* Keep additions whose managed parent is unavailable explicitly unapplied, including descendants, instead of throwing an invariant exception or creating orphans. Retain the subtree for retry and continue unrelated valid additions; report the missing parent context. ([#1094])

* Warn when a built place contains StarterCharacterScripts without StarterPlayerScripts. Inspect the final tree, including model sources, and document explicit typed container declarations without inserting instances or changing defaults. ([#461])

* Document local Windows CLI/Studio setup, version and capability distinctions, the fork8.0.0 Rokit pin, explicit quoted paths and command-not-found recovery. Include a clean-PATH/spaced-path verification helper without changing installed tools or user PATH. ([#1313], [#1339])

* Clarify stored Model.Scale and baked geometry semantics, with native export/reify/update and JSON/XML/binary roundtrip coverage. Preserve valid exports without double scaling; document equivalent native exported state. ([#1343])

* Honor class-specific property authority during explicit syncback: retain filesystem-owned properties and native source bytes while exporting Studio-owned descendants. Reject changed filesystem structure and unsafe reference preservation before writes; keep existing format and layout restrictions. ([#1301])

* Preserve Studio-owned class properties during initial connection and live file updates through negotiated `classAuthorityV1`. Retain instance/reference identities, require explicit review for authority changes or destructive replacement, and protect serialized/topology recovery. Legacy clients refuse incompatible sessions. ([#1301])

* Preserve inherited context for CSV snapshots without changing the CSV middleware. Keep custom CSV routes and instance identity on direct watch updates; apply inherited unknown-descendant preservation while retaining explicit metadata and shared-rule precedence. ([#1036])

* Add optional inherited `syncAuthority` class policies. Explicit matching policies block legacy writes and Source-only receipts; check current project state before legacy mutations. Preserve nested overrides, embedded/CSV authority, offline build/stdin/syncback and JSONC destinations. ([#1301])

* Add validated reusable class-authority types with exact/nearest-superclass precedence and strict configuration validation. ([#1301])

* Remove deleted files and directory subtrees from their parent listings in the in-memory filesystem, matching native deletion and allowing clean recreation.

* Repair moved/detached managed instances, reconstruct uncontested deleted subtrees and rebind exact copies under opt-in adoption before live updates. Preserve references, unknown descendants, selection and Undo; refuse ambiguous or dirty-source recovery before mutation and retain reconnect review for dirty copies. ([#296])

* Apply project ignore globs and negations to absolute source bindings outside the project directory, including wildcard rules on other drives. Retain project-relative `../` patterns on the same drive. ([#907])

* Add shared `syncRules.defaults` properties, attributes, tags and metadata, with independent routing, explicit override precedence and per-key removal masks. Preserve rule authority through syncback without per-file default copies; retain JSONC paths and reject lossy binary property columns. Apply live sync policy metadata without replacing healthy instances. ([#1036])

* Add opt-in adoption of saved, untracked Studio instances during live sync after unique readable-property/subtree/reference checks. Preserve identities and unknown descendants; stop ambiguous conflicts before mutations or fallback. Serialization-only values remain unchanged and explicitly unverified. Preserve manual edits when refusing a patch. ([#673], [#1303])

* Confirm opt-in two-way Source writes with bounded correlated persistence receipts and file readback; preserve newer local edits, expose conflicts for explicit review, and require confirmation after uncertain reconnects. Legacy servers report queued-only writes. General instance/property ownership remains separate. ([#293], [#295])

* Extract reusable project tree, path and reflected value types into the local `rojo-project-types` crate, without filesystem, CLI, network or place-serializer dependencies. Preserve existing `librojo` exports and exact Rust type identity through re-exports. ([#508])

* Make CLI and HTTP dependencies optional through a default-enabled `cli` feature. `default-features = false` retains the build, project, snapshot, syncback and wire-data APIs without the executable, transport or embedded-plugin build; standard CLI builds remain enabled. ([#1003])

* Recognize `init.project.json/jsonc` directory entries only with explicit V2, report conflicting entries, retain default-project priority and explicit-file selection, and refresh directory roles after live alias/version changes without changing syncback destinations. ([#1323])

* Add optional project `$version` 1/2. Explicit V2 defaults to modern scripts and inherited unknown-descendant preservation; unversioned projects retain their defaults, explicit overrides win, and reload/syncback retain the chosen semantics without migrating files. ([#706])

* Add opt-in loopback automatic debugger plans with per-connection Studio consent, managed-source versions, bounded native stacks/locals/table inspection, typed numeric filters/evaluation and predetermined steps; revoke points/callbacks on lifecycle changes and expose bounded immutable reports. ([#1279])

* Add the Windows `open` workflow to build or copy a place, serve on loopback and launch Studio with a one-session marker, guarded plugin installation and mandatory initial-sync confirmation. Preserve original places, existing output/plugin files and edited temporary places. ([#305])

* Add explicit `textModule` sync rules for UTF-8 files, generating ModuleScripts that return exact text; support byte-preserving syncback/live writes of canonical literals and reject arbitrary code or incompatible layouts before writing. Existing CSV and text defaults remain unchanged. ([#1204])

* Add opt-in inherited `ignoreEmptyDirectories` filtering for implicit physically empty directory trees, preserving explicit bindings and file markers, and keeping live/watch trees current after last-file removal and recreation. ([#969])

* Add opt-in `build --compress lz4|zstd|none` for binary outputs, including watch, stdin and existing-place updates, while retaining LZ4 defaults and rejecting explicit compression for XML before writing. ([#1026])

* Read project JSON/JSONC from stdin with `build -`, resolve sources from the current directory, and retain the in-memory project during watch builds without writing a temporary project file. ([#626])

* Expose non-archivable `game.Rojo` discovery with per-plugin consent, scoped connection/status/settings/notification/context handles, explicit socket readiness and revocation/unload cleanup. ([#638], [#321])

* Add opt-in Studio output forwarding with severity filters, bounded batches and current Lua source file/line mapping; stop forwarding on disconnect and retain compatibility with older servers/plugins. ([#1277])

* Accept project stems such as `game` for `game.project.json` or `game.project.jsonc` across project-reading CLI commands, retaining existing path precedence and output/input filenames. ([#895])

* Use Open Cloud for existing model updates and place publishing, with API-key validation, model operation polling and correct binary content types; deprecate and ignore cookie authentication without reading installed cookies. ([#1263], [#1141], [#926])

* Add `build --input` for scoped updates of existing binary/XML places, preserving unmentioned properties and unowned areas, maintaining references and replacing output only after successful serialization. ([#1007], [#287])

* Add `syncback --watch` with explicit repeated-write consent, coalesced input saves, recovery after incomplete input, fresh project reads and protection against overwriting its input. ([#1229])

* Resolve relative syncback root directories from the project folder, preserving new children independently of the current working directory. ([#1229])

* Isolate serialized fallback models so one broken asset does not block healthy replacements; restore references, preserve failed changes, and retain legacy-server support. ([#1115])

* Disconnect when Studio changes to a place ID excluded by the connected project, reusing connection-time allow/deny checks and preventing further patch or write work; allowed and unrestricted ID changes remain connected. ([#1308])
* Warn with instance and property paths when live sync skips a requested unreadable property, including guidance for serialization-only place settings. ([#874])
* Wrap the last-update control below the version header in narrow plugin panels, returning to a single row when widened and accounting for longer prerelease version labels. ([#1158])
* Stop patch-row animations and delayed expansion clicks after virtualized rows unmount, including removal during a height update, preventing Roact updates to destroyed UI. ([#759])
* Skip queued tooltips whose trigger was removed during text measurement or virtualized scrolling, while continuing to render attached controls. ([#756])
* Add a failures-only patch view with ancestor paths and failed properties, allowing quick inspection among many successful changes and a return to the full change list. ([#1230])
* Add Cancel while connecting and stop pending startup work, preventing late responses or confirmation callbacks from applying changes, reconnecting or opening sockets after cancellation. ([#1171])
* Ignore repeated Connect actions and repeated session starts, preventing duplicate API connections and sync lock claims while allowing a fresh retry after immediate startup failure. ([#1046])
* Support explicit native Script/LocalScript metadata and faithful Lua syncback across emitter modes and script rules, preserving class, RunContext, source, children, IDs and existing Lua/JSONC paths; reject incompatible bindings and unsupported layout migrations before writes. ([#791], [#1078], [#1166])
* Support nonempty directory arrays in `$path` for read-only sourcemaps, with canonical deduplication, recursive conflict-checked merges and watch regeneration across sources; build, serve and syncback reject array ownership. ([#1047])
* Add loaded-project `build_project` and typed `write_project` library APIs, preserving caller edits, reference resolution and the CLI serializers without starting a server. ([#1001])
* Add opt-in inherited `$ignoreUnknownDescendants` preservation with local metadata overrides, and warn with counts and paths before initial sync removes unknown Studio instances. Existing defaults and managed deletions are preserved. ([#716], [#919])
* Accept matching class declarations in project nodes and filesystem metadata while continuing to reject conflicting classes. ([#197])
* Warn with file and node paths for ignored model/metadata fields, including a correction for project-style `$properties`; retain supported tag handling and JSONC metadata paths. ([#1017])
* Accept unsigned 32-bit integer enum values in project, model and metadata properties, retaining named/qualified enums and rejecting invalid numeric values. ([#1139])
* Bound HTTP API request bodies to 64 MiB by default, with a positive `serve --max-request-body-bytes` override and structured HTTP 413 errors before parsing oversized bodies. ([#1275])
* Expose normalized source file paths on API reads with `?showPaths=true`, including embedded model children, while retaining the default response and public Rust structs. ([#335])
* Add opt-in `sourcemap --relative-to-output` for output-directory-relative file paths, including watch mode, while preserving the default project-relative paths. ([#999])
* Add opt-in `sourcemap --include-run-context` with reflected names and raw values for Script nodes, including property-only watch updates and default/unknown values. Existing class names and default output stay unchanged. ([#1317])
* Preserve Studio-only script descendants when toggling `emitLegacyScripts` for an unchanged, unambiguous script source. ([#925])
* Batch rapid `build --watch` changes into a bounded collection window to avoid exporting once per changed file. ([#577])
* Avoid accumulating duplicate ignore rules when projects are re-snapshotted, preserving negation priority and each rule's base path. ([#1018])
* Support TweenInfo attributes using the local codec fork, preserving mixed attributes and avoiding repeated sync differences. Both server and plugin must support the negotiated extension. ([#1336])
* Preserve legacy MeshPart mass when binary output combines models with and without stored volume data. ([#1312])
* Keep live reference properties consistent with the final tree when targets are added, changed, removed, or ambiguous, and correctly apply explicit nil references. ([#1292])
* Avoid recurring plugin batcher errors for reference and unsupported property encoders; use tracked IDs for reference encoding and report untracked targets without blocking later source updates. ([#1304])
* Preserve parents of Init-named scripts and localization tables during syncback by using their directory representation for reserved filenames. ([#1195])
* Preserve existing directory contents and reject file collisions in memofs in-memory `create_dir_all`. ([#1166])
* Allow duplicate Studio-only children beneath project nodes without `$path` during syncback, while still rejecting ambiguous or missing managed children. ([#1180])
* Keep new children inside mapped model files during syncback instead of attempting to write paths beneath those files. ([#1225])
* Serialize syncback metadata and JSON-model properties and attributes in stable lexical order. ([#1245])
* Warn at session startup when the root project maps the same canonical `$path` to multiple nodes, retaining intentional aliases. ([#475])
* Added opt-in filesystem operation and watcher diagnostics through the `memofs` log target. ([#747])
* Reject invalid Windows build output names instead of reporting successful builds into alternate data streams or device files. ([#660])
* Report an actionable error before attempting a legacy place upload, directing users to Open Cloud authentication. ([#931])
* Include active filesystem event, project, and build output paths in crash diagnostics. ([#932])
* Report a contextual error for directory paths without a name, such as drive roots, instead of panicking during snapshot selection. ([#932])
* Return a contextual API error when serializing an instance fails, keeping the serve session usable. ([#932])
* Reject API writes with an actionable error if the change processor has stopped, instead of panicking on a closed channel. ([#932])
* Report the source and project paths when syncback cannot determine the root path's middleware, instead of panicking. ([#932])
* Preserve known instance identities when reconnecting to the same live server, so offline class changes and deletions do not leave stale or duplicate instances. Unknown-instance rules remain unchanged. ([#1009])
* Fixed delayed live-sync additions crashing after deletion or including later changes and duplicate children. ([#1070])
* Fixed syncback crashing when removing an embedded child from a model referenced by a project node. ([#1121])
* Fixed a subscription race that could leave connected clients waiting until another change arrived. ([#1136])
* Explain unexpected plugin disconnections with the server address and restart/reconnect guidance, retaining transport error details. ([#1240])
* Fixed plugin source updates writing to a directory instead of its init script, and publishing changes when writing the source failed. ([#1206])
* Return structured request errors instead of panicking when an API request body is aborted or cannot be read. ([#1275])
* Avoided redundant metadata patches for unchanged instances with `Rojo_Id` attributes. ([#1292])
* Fixed lost filesystem watches after failed registration or deletion and recreation of mapped directories. ([#1309])
* Fixed live-sync crashes when event targets or their parent directories disappear during rapid edits, and when the root source is temporarily removed. ([#1314])
* Reset removed properties in Studio through a negotiated live-sync extension, including reference properties, attributes, and tags. Both the server and plugin must support the extension. ([#1332])
* LocalizationTables no longer report as changed when nothing changes. [(#1333)]

[8.0.0]: https://github.com/jonasvq/rojo/releases/tag/v8.0.0

[#164]: https://github.com/rojo-rbx/rojo/issues/164
[#166]: https://github.com/rojo-rbx/rojo/issues/166
[#197]: https://github.com/rojo-rbx/rojo/issues/197
[#293]: https://github.com/rojo-rbx/rojo/issues/293
[#295]: https://github.com/rojo-rbx/rojo/issues/295
[#296]: https://github.com/rojo-rbx/rojo/issues/296
[#305]: https://github.com/rojo-rbx/rojo/issues/305
[#321]: https://github.com/rojo-rbx/rojo/issues/321
[#335]: https://github.com/rojo-rbx/rojo/issues/335
[#461]: https://github.com/rojo-rbx/rojo/issues/461
[#475]: https://github.com/rojo-rbx/rojo/issues/475
[#508]: https://github.com/rojo-rbx/rojo/issues/508
[#577]: https://github.com/rojo-rbx/rojo/issues/577
[#626]: https://github.com/rojo-rbx/rojo/issues/626
[#638]: https://github.com/rojo-rbx/rojo/issues/638
[#644]: https://github.com/rojo-rbx/rojo/issues/644
[#660]: https://github.com/rojo-rbx/rojo/issues/660
[#673]: https://github.com/rojo-rbx/rojo/issues/673
[#706]: https://github.com/rojo-rbx/rojo/issues/706
[#716]: https://github.com/rojo-rbx/rojo/issues/716
[#747]: https://github.com/rojo-rbx/rojo/issues/747
[#756]: https://github.com/rojo-rbx/rojo/issues/756
[#759]: https://github.com/rojo-rbx/rojo/issues/759
[#791]: https://github.com/rojo-rbx/rojo/issues/791
[#812]: https://github.com/rojo-rbx/rojo/issues/812
[#874]: https://github.com/rojo-rbx/rojo/issues/874
[#906]: https://github.com/rojo-rbx/rojo/issues/906
[#907]: https://github.com/rojo-rbx/rojo/issues/907
[#919]: https://github.com/rojo-rbx/rojo/issues/919
[#925]: https://github.com/rojo-rbx/rojo/issues/925
[#931]: https://github.com/rojo-rbx/rojo/issues/931
[#932]: https://github.com/rojo-rbx/rojo/issues/932
[#969]: https://github.com/rojo-rbx/rojo/issues/969
[#981]: https://github.com/rojo-rbx/rojo/issues/981
[#999]: https://github.com/rojo-rbx/rojo/issues/999
[#1003]: https://github.com/rojo-rbx/rojo/issues/1003
[#1009]: https://github.com/rojo-rbx/rojo/issues/1009
[#1016]: https://github.com/rojo-rbx/rojo/issues/1016
[#1001]: https://github.com/rojo-rbx/rojo/issues/1001
[#1017]: https://github.com/rojo-rbx/rojo/issues/1017
[#1018]: https://github.com/rojo-rbx/rojo/issues/1018
[#1026]: https://github.com/rojo-rbx/rojo/issues/1026
[#1046]: https://github.com/rojo-rbx/rojo/issues/1046
[#1047]: https://github.com/rojo-rbx/rojo/issues/1047
[#1070]: https://github.com/rojo-rbx/rojo/issues/1070
[#1078]: https://github.com/rojo-rbx/rojo/issues/1078
[#1094]: https://github.com/rojo-rbx/rojo/issues/1094
[#1121]: https://github.com/rojo-rbx/rojo/issues/1121
[#1136]: https://github.com/rojo-rbx/rojo/issues/1136
[#1139]: https://github.com/rojo-rbx/rojo/issues/1139
[#1206]: https://github.com/rojo-rbx/rojo/issues/1206
[#1158]: https://github.com/rojo-rbx/rojo/issues/1158
[#1166]: https://github.com/rojo-rbx/rojo/issues/1166
[#1171]: https://github.com/rojo-rbx/rojo/issues/1171
[#1180]: https://github.com/rojo-rbx/rojo/issues/1180
[#1195]: https://github.com/rojo-rbx/rojo/issues/1195
[#1221]: https://github.com/rojo-rbx/rojo/issues/1221
[#1225]: https://github.com/rojo-rbx/rojo/issues/1225
[#1240]: https://github.com/rojo-rbx/rojo/issues/1240
[#1230]: https://github.com/rojo-rbx/rojo/issues/1230
[#1245]: https://github.com/rojo-rbx/rojo/issues/1245
[#1275]: https://github.com/rojo-rbx/rojo/issues/1275
[#1277]: https://github.com/rojo-rbx/rojo/issues/1277
[#1283]: https://github.com/rojo-rbx/rojo/issues/1283
[#1292]: https://github.com/rojo-rbx/rojo/issues/1292
[#1299]: https://github.com/rojo-rbx/rojo/issues/1299
[#1301]: https://github.com/rojo-rbx/rojo/issues/1301
[#1302]: https://github.com/rojo-rbx/rojo/issues/1302
[#1303]: https://github.com/rojo-rbx/rojo/issues/1303
[#1307]: https://github.com/rojo-rbx/rojo/issues/1307
[#1304]: https://github.com/rojo-rbx/rojo/issues/1304
[#1308]: https://github.com/rojo-rbx/rojo/issues/1308
[#1309]: https://github.com/rojo-rbx/rojo/issues/1309
[#1312]: https://github.com/rojo-rbx/rojo/issues/1312
[#1313]: https://github.com/rojo-rbx/rojo/issues/1313
[#1314]: https://github.com/rojo-rbx/rojo/issues/1314
[#1323]: https://github.com/rojo-rbx/rojo/issues/1323
[#1317]: https://github.com/rojo-rbx/rojo/issues/1317
[#1339]: https://github.com/rojo-rbx/rojo/issues/1339
[#1343]: https://github.com/rojo-rbx/rojo/issues/1343
[#1332]: https://github.com/rojo-rbx/rojo/issues/1332
[#1333]: https://github.com/rojo-rbx/rojo/pull/1333
[#1336]: https://github.com/rojo-rbx/rojo/issues/1336

## [7.7.1] (October 1st, 2026)

* Fixed `$path` values that point outside the project folder failing to match `syncRule`s on Windows, which broke `rojo sourcemap` with a "could not be turned into a Roblox Instance" error. ([#1290])
* Fixed `rojo serve` silently stopping syncing file changes on Windows when the served project path was a verbatim (`\\?\`) path, because tree paths and file-watcher event paths were canonicalized to different forms. ([#1290])
* Fixed `rojo sourcemap --absolute` emitting verbatim (`\\?\`) paths on Windows, which broke require types in luau-lsp. ([#1290])
* The plugin now disables the `Check for Updates` setting if you block access to `api.github.com`. ([#1297])

[7.7.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.7.1
[#1290]: https://github.com/rojo-rbx/rojo/pull/1290
[#1297]: https://github.com/rojo-rbx/rojo/pull/1297

## [7.7.0] (July 1st, 2026)

* `inf` and `nan` values in properties are now synced ([#1176])
* Fixed a bug caused by having reference properties (such as `ObjectValue.Value`) that point to an Instance not included in syncback. ([#1179])
* Fixed instance replacement fallback failing when too many instances needed to be replaced. ([#1192])
* Added actors and bindable/remote event/function variants to be synced back as JSON files. ([#1199])
* Fixed a bug where MacOS paths weren't being handled correctly. ([#1201])
* Fixed a bug where the notification timeout thread would fail to cancel on unmount ([#1211])
* Added a "Forget" option to the sync reminder notification to avoid being reminded for that place in the future ([#1215])
* Improves relative path calculation for sourcemap generation to avoid issues with Windows UNC paths. ([#1217])
* Fixed missing support for init.plugin.lua and init.plugin.luau. ([#1252])
* Add support for gitignore-style negation in `globIgnorePaths` and syncback's `ignorePaths` ([#1256])
* Fixed the sync fallback scrambling sibling order; replacements are now re-parented ancestors-first and in their original child order. ([#1265])
* Instances that share a name and class are now robustly matched on resync by comparing their properties, instead of relying on child order alone. ([#1266])
* Rojo now reports a clear error instead of panicking in several cases, including when the `serve` port is already in use, when a synced file is read-only or locked, when the filesystem watcher can't be created, and when the working directory is inaccessible. ([#1267])
* Fixed `/api/serialize` returning success when a requested instance ID is missing from the serve session tree. ([#1272])
* `rojo serve` now validates the `Host`/`Origin` headers to protect the local/private server against DNS rebinding, gates `/api/open` to local clients, and warns when bound to a network-reachable address. The accepted hosts can be extended with the `--allowed-hosts` option or a project's `serveAllowedHosts` field, for example to reach a network-exposed server by hostname. ([#1270])
* Fixed syncback not removing stale `$properties` entries when Studio resets a property to its engine default. ([#1244])

[7.7.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.7.0
[#1176]: https://github.com/rojo-rbx/rojo/pull/1176
[#1179]: https://github.com/rojo-rbx/rojo/pull/1179
[#1192]: https://github.com/rojo-rbx/rojo/pull/1192
[#1199]: https://github.com/rojo-rbx/rojo/pull/1199
[#1201]: https://github.com/rojo-rbx/rojo/pull/1201
[#1211]: https://github.com/rojo-rbx/rojo/pull/1211
[#1215]: https://github.com/rojo-rbx/rojo/pull/1215
[#1217]: https://github.com/rojo-rbx/rojo/pull/1217
[#1252]: https://github.com/rojo-rbx/rojo/pull/1252
[#1256]: https://github.com/rojo-rbx/rojo/pull/1256
[#1265]: https://github.com/rojo-rbx/rojo/pull/1265
[#1266]: https://github.com/rojo-rbx/rojo/pull/1266
[#1267]: https://github.com/rojo-rbx/rojo/pull/1267
[#1272]: https://github.com/rojo-rbx/rojo/pull/1272
[#1270]: https://github.com/rojo-rbx/rojo/pull/1270
[#1244]: https://github.com/rojo-rbx/rojo/pull/1244

## [7.7.0-rc.1] (November 27th, 2025)

* Fixed a bug where passing `--skip-git` to `rojo init` would still create a file named `gitignore.txt` ([#1172])
* A new command `rojo syncback` has been added. It can be used as `rojo syncback [path to project] --input [path to file]`. ([#937])
 	This command takes a Roblox file and pulls Instances out of it and places them in the correct position in the provided project.
    Syncback is primarily controlled by the project file. Any Instances who are either referenced in the project file or a descendant
    of one that is will be placed in an appropriate location.

    In addition, a new field has been added to project files, `syncbackRules` to control how it behaves:

    ```json
    {
        "syncbackRules": {
            "ignoreTrees": [
                "ServerStorage/ImportantSecrets",
            ],
            "ignorePaths": [
                "src/ServerStorage/Secrets/*"
            ],
            "ignoreProperties": {
                "BasePart": ["Color"]
            },
            "syncCurrentCamera": false,
            "syncUnscriptable": true,
        }
    }
    ```

    A brief explanation of each field:

    - `ignoreTrees` is a list of paths in the **roblox file** that should be ignored
    - `ignorePaths` is a list of paths in the **file system** that should be ignored
    - `ignoreProperties` is a list of properties that won't be synced back
    - `syncCurrentCamera` is a toggle for whether to sync back the Workspace's CurrentCamera. Defaults to `false`.
    - `syncUnscriptable` is a toggle for whether to sync back properties that cannot be set by the Roblox Studio plugin. Defaults to `true`.

* Fixed bugs and improved performance & UX for the script diff viewer ([#994])
* Rebuilt the internal communication between the server and plugin to use [websockets](https://devforum.roblox.com/t/websockets-support-in-studio-is-now-available/4021932/1) instead of [long polling](https://en.wikipedia.org/wiki/Push_technology#Long_polling) ([#1142])
* Added support for `.jsonc` files for all JSON-related files (e.g. `.project.jsonc` and `.meta.jsonc`) to accompany JSONC support ([#1159])

[7.7.0-rc.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.7.0-rc.1
[#937]: https://github.com/rojo-rbx/rojo/pull/937
[#994]: https://github.com/rojo-rbx/rojo/pull/994
[#1142]: https://github.com/rojo-rbx/rojo/pull/1142
[#1159]: https://github.com/rojo-rbx/rojo/pull/1159
[#1172]: https://github.com/rojo-rbx/rojo/pull/1172

## [7.6.1] (November 6th, 2025)

* Fixed a bug where the last sync timestamp was not updating correctly in the plugin ([#1132])
* Improved the reliability of sync replacements by adding better error handling and recovery ([#1135])
* Small improvements to stability when syncing massive projects ([#1140])
* Added support for JSON comments and trailing commas in project, meta, and model json files ([#1144])
* Added `sourcemap.json` into the default `.gitignore` files ([#1145])

[7.6.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.6.1
[#1132]: https://github.com/rojo-rbx/rojo/pull/1132
[#1135]: https://github.com/rojo-rbx/rojo/pull/1135
[#1140]: https://github.com/rojo-rbx/rojo/pull/1140
[#1144]: https://github.com/rojo-rbx/rojo/pull/1144
[#1145]: https://github.com/rojo-rbx/rojo/pull/1145

## [7.6.0] (October 10th, 2025)

* Added flag to `rojo init` to skip initializing a git repository ([#1122])
* Added fallback method for when an Instance can't be synced through normal means ([#1030])

    This should make it possible to sync `MeshParts` and `Unions`!

    The fallback involves deleting and recreating Instances. This will break properties that reference them that Rojo does not know about, so be weary.

* Add auto-reconnect and improve UX for sync reminders ([#1096])
* Add support for syncing `yml` and `yaml` files (behaves similar to JSON and TOML) ([#1093])
* Fixed colors of Table diff ([#1084])
* Fixed `sourcemap` command outputting paths with OS-specific path separators ([#1085])
* Fixed nil -> nil properties showing up as failing to sync in plugin's patch visualizer ([#1081])
* Changed the background of the server's in-browser UI to be gray instead of white ([#1080])
* Fixed `Auto Connect Playtest Server` no longer functioning due to Roblox change ([#1066])
* Added an update indicator to the version header when a new version of the plugin is available. ([#1069])
* Added `--absolute` flag to the sourcemap subcommand, which will emit absolute paths instead of relative paths. ([#1092])
* Fixed applying `gameId` and `placeId` before initial sync was accepted ([#1104])

[7.6.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.6.0
[#1030]: https://github.com/rojo-rbx/rojo/pull/1030
[#1036]: https://github.com/rojo-rbx/rojo/issues/1036
[#1066]: https://github.com/rojo-rbx/rojo/pull/1066
[#1069]: https://github.com/rojo-rbx/rojo/pull/1069
[#1080]: https://github.com/rojo-rbx/rojo/pull/1080
[#1081]: https://github.com/rojo-rbx/rojo/pull/1081
[#1084]: https://github.com/rojo-rbx/rojo/pull/1084
[#1085]: https://github.com/rojo-rbx/rojo/pull/1085
[#1092]: https://github.com/rojo-rbx/rojo/pull/1092
[#1093]: https://github.com/rojo-rbx/rojo/pull/1093
[#1096]: https://github.com/rojo-rbx/rojo/pull/1096
[#1104]: https://github.com/rojo-rbx/rojo/pull/1104
[#1122]: https://github.com/rojo-rbx/rojo/pull/1122

## [7.5.1] (April 25th, 2025)

* Fixed output spam related to `Instance.Capabilities` in the plugin

[7.5.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.5.1

## [7.5.0] (April 25th, 2025)

* Fixed an edge case that caused model pivots to not be built correctly in some cases ([#1027])
* Add `blockedPlaceIds` project config field to allow blocking place ids from being live synced ([#1021])
* Adds support for `.plugin.lua(u)` files - this applies the `Plugin` RunContext. ([#1008])
* Added support for Roblox's `Content` type. This replaces the old `Content` type with `ContentId` to reflect Roblox's change.

    If you were previously using the fully-qualified syntax for `Content` you will need to switch it to `ContentId`.
* Added support for `Enum` attributes
* Significantly improved performance of `.rbxm` parsing
* Support for a `$schema` field in all special JSON files (`.project.json`, `.model.json`, and `.meta.json`) ([#974])
* Projects may now manually link `Ref` properties together using `Attributes`. ([#843])

    This has two parts: using `id` or `$id` in JSON files or a `Rojo_Target` attribute, an Instance
    is given an ID. Then, that ID may be used elsewhere in the project to point to an Instance
    using an attribute named `Rojo_Target_PROP_NAME`, where `PROP_NAME` is the name of a property.

    As an example, here is a `model.json` for an ObjectValue that refers to itself:

    ```json
    {
        "id": "arbitrary string",
        "attributes": {
            "Rojo_Target_Value": "arbitrary string"
        }
    }
    ```

    This is a very rough implementation and the usage will become more ergonomic
    over time.

* Updated Undo/Redo history to be more robust ([#915])
* Added popout diff visualizer for table properties like Attributes and Tags ([#834])
* Updated Theme to use Studio colors ([#838])
* Improved patch visualizer UX ([#883])
* Added update notifications for newer compatible versions in the Studio plugin. ([#832])
* Added experimental setting for Auto Connect in playtests ([#840])
* Improved settings UI ([#886])
* `Open Scripts Externally` option can now be changed while syncing ([#911])
* The sync reminder notification will now tell you what was last synced and when ([#987])
* Fixed notification and tooltip text sometimes getting cut off ([#988])
* Projects may now specify rules for syncing files as if they had a different file extension. ([#813])

    This is specified via a new field on project files, `syncRules`:

    ```json
    {
      "syncRules": [
        {
          "pattern": "*.foo",
          "use": "text",
          "exclude": "*.exclude.foo",
        },
        {
          "pattern": "*.bar.baz",
          "use": "json",
          "suffix": ".bar.baz",
        },
      ],
      "name": "SyncRulesAreCool",
      "tree": {
        "$path": "src"
      }
    }
    ```

    The `pattern` field is a glob used to match the sync rule to files. If present, the `suffix` field allows you to specify parts of a file's name get cut off by Rojo to name the Instance, including the file extension. If it isn't specified, Rojo will only cut off the first part of the file extension, up to the first dot.

    Additionally, the `exclude` field allows files to be excluded from the sync rule if they match a pattern specified by it. If it's not present, all files that match `pattern` will be modified using the sync rule.

    The `use` field corresponds to one of the potential file type that Rojo will currently include in a project. Files that match the provided pattern will be treated as if they had the file extension for that file type.

    | `use` value    | file extension  |
    |:---------------|:----------------|
    | `serverScript` | `.server.lua`   |
    | `clientScript` | `.client.lua`   |
    | `moduleScript` | `.lua`          |
    | `json`         | `.json`         |
    | `toml`         | `.toml`         |
    | `csv`          | `.csv`          |
    | `text`         | `.txt`          |
    | `jsonModel`    | `.model.json`   |
    | `rbxm`         | `.rbxm`         |
    | `rbxmx`        | `.rbxmx`        |
    | `project`      | `.project.json` |
    | `ignore`       | None!           |

    Additionally, there are `use` values for specific script types ([#909]):

    | `use` value              | script type                            |
    |:-------------------------|:---------------------------------------|
    | `legacyServerScript`     | `Script` with `Enum.RunContext.Legacy` |
    | `legacyClientScript`     | `LocalScript`                          |
    | `runContextServerScript` | `Script` with `Enum.RunContext.Server` |
    | `runContextClientScript` | `Script` with `Enum.RunContext.Client` |
    | `pluginScript`           | `Script` with `Enum.RunContext.Plugin` |

    **All** sync rules are reset between project files, so they must be specified in each one when nesting them. This is to ensure that nothing can break other projects by changing how files are synced!

[7.5.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.5.0
[#813]: https://github.com/rojo-rbx/rojo/pull/813
[#832]: https://github.com/rojo-rbx/rojo/pull/832
[#834]: https://github.com/rojo-rbx/rojo/pull/834
[#838]: https://github.com/rojo-rbx/rojo/pull/838
[#840]: https://github.com/rojo-rbx/rojo/pull/840
[#843]: https://github.com/rojo-rbx/rojo/pull/843
[#883]: https://github.com/rojo-rbx/rojo/pull/883
[#886]: https://github.com/rojo-rbx/rojo/pull/886
[#909]: https://github.com/rojo-rbx/rojo/pull/909
[#911]: https://github.com/rojo-rbx/rojo/pull/911
[#915]: https://github.com/rojo-rbx/rojo/pull/915
[#974]: https://github.com/rojo-rbx/rojo/pull/974
[#987]: https://github.com/rojo-rbx/rojo/pull/987
[#988]: https://github.com/rojo-rbx/rojo/pull/988
[#1008]: https://github.com/rojo-rbx/rojo/pull/1008
[#1021]: https://github.com/rojo-rbx/rojo/pull/1021
[#1027]: https://github.com/rojo-rbx/rojo/pull/1027

## [7.4.4] (August 22nd, 2024)

* Fixed issue with reading attributes from `Lighting` in new place files
* `Instance.Archivable` will now default to `true` when building a project into a binary (`rbxm`/`rbxl`) file rather than `false`.

[7.4.4]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.4

## [7.4.3] (August 6th, 2024)

* Fixed issue with building binary files introduced in 7.4.2
* Fixed `value of type nil cannot be converted to number` warning spam in output. [#955]

[7.4.3]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.3
[#955]: https://github.com/rojo-rbx/rojo/pull/955

## [7.4.2] (July 23, 2024)

* Added Never option to Confirmation ([#893])
* Fixed removing trailing newlines ([#903])
* Updated the internal property database, correcting an issue with `SurfaceAppearance.Color` that was reported [here][Surface_Appearance_Color_1] and [here][Surface_Appearance_Color_2] ([#948])

[7.4.2]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.2
[#893]: https://github.com/rojo-rbx/rojo/pull/893
[#903]: https://github.com/rojo-rbx/rojo/pull/903
[#948]: https://github.com/rojo-rbx/rojo/pull/948
[Surface_Appearance_Color_1]: https://devforum.roblox.com/t/jailbreak-custom-character-turned-shiny-black-no-texture/3075563
[Surface_Appearance_Color_2]: https://devforum.roblox.com/t/surfaceappearance-not-displaying-correctly/3075588

## [7.4.1] (February 20, 2024)

* Made the `name` field optional on project files ([#870])

    Files named `default.project.json` inherit the name of the folder they're in and all other projects
    are named as expect (e.g. `foo.project.json` becomes an Instance named `foo`)

    There is no change in behavior if `name` is set.
* Fixed incorrect results when building model pivots ([#865])
* Fixed incorrect results when serving model pivots ([#868])
* Rojo now converts any line endings to LF, preventing spurious diffs when syncing Lua files on Windows ([#854])
* Fixed Rojo plugin failing to connect when project contains certain unreadable properties ([#848])
* Fixed various cases where patch visualizer would not display sync failures ([#845], [#844])
* Fixed http error handling so Rojo can be used in Github Codespaces ([#847])

[7.4.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.1
[#844]: https://github.com/rojo-rbx/rojo/pull/844
[#845]: https://github.com/rojo-rbx/rojo/pull/845
[#847]: https://github.com/rojo-rbx/rojo/pull/847
[#848]: https://github.com/rojo-rbx/rojo/pull/848
[#854]: https://github.com/rojo-rbx/rojo/pull/854
[#865]: https://github.com/rojo-rbx/rojo/pull/865
[#868]: https://github.com/rojo-rbx/rojo/pull/868
[#870]: https://github.com/rojo-rbx/rojo/pull/870

## [7.4.0] (January 16, 2024)

* Improved the visualization for array properties like Tags ([#829])
* Significantly improved performance of `rojo serve`, `rojo build --watch`, and `rojo sourcemap --watch` on macOS. ([#830])
* Changed `*.lua` files that init command generates to `*.luau` ([#831])
* Does not remind users to sync if the sync lock is claimed already ([#833])

[7.4.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.0
[#829]: https://github.com/rojo-rbx/rojo/pull/829
[#830]: https://github.com/rojo-rbx/rojo/pull/830
[#831]: https://github.com/rojo-rbx/rojo/pull/831
[#833]: https://github.com/rojo-rbx/rojo/pull/833

## [7.4.0-rc3] (October 25, 2023)

* Changed `sourcemap --watch` to only generate the sourcemap when it's necessary ([#800])
* Switched script source property getter and setter to `ScriptEditorService` methods ([#801])

    This ensures that the script editor reflects any changes Rojo makes to a script while it is open in the script editor.

* Fixed issues when handling `SecurityCapabilities` values ([#803], [#807])
* Fixed Rojo plugin erroring out when attempting to sync attributes with invalid names ([#809])

[7.4.0-rc3]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.0-rc3
[#800]: https://github.com/rojo-rbx/rojo/pull/800
[#801]: https://github.com/rojo-rbx/rojo/pull/801
[#803]: https://github.com/rojo-rbx/rojo/pull/803
[#807]: https://github.com/rojo-rbx/rojo/pull/807
[#809]: https://github.com/rojo-rbx/rojo/pull/809

## [7.4.0-rc2] (October 3, 2023)

* Fixed bug with parsing version for plugin validation ([#797])

[7.4.0-rc2]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.0-rc2
[#797]: https://github.com/rojo-rbx/rojo/pull/797

## [7.4.0-rc1] (October 3, 2023)

### Additions

#### Project format

* Added support for `.toml` files to `$path` ([#633])
* Added support for `Font` and `CFrame` attributes ([rbx-dom#299], [rbx-dom#296])
* Added the `emitLegacyScripts` field to the project format ([#765]). The behavior is outlined below:

    | `emitLegacyScripts` Value | Action Taken by Rojo                                                                                             |
    |---------------------------|------------------------------------------------------------------------------------------------------------------|
    | false                     | Rojo emits Scripts with the appropriate `RunContext` for `*.client.lua` and `*.server.lua` files in the project. |
    | true   (default)          | Rojo emits LocalScripts and Scripts with legacy `RunContext` (same behavior as previously).                      |

    It can be used like this:

    ```json
    {
      "emitLegacyScripts": false,
      "name": "MyCoolRunContextProject",
      "tree": {
        "$path": "src"
      }
    }
    ```

* Added `Terrain` classname inference, similar to services ([#771])

    `Terrain` may now be defined in projects without using `$className`:

    ```json
    "Workspace": {
      "Terrain": {
        "$path": "path/to/terrain.rbxm"
      }
    }
    ```

* Added support for `Terrain.MaterialColors` ([#770])

    `Terrain.MaterialColors` is now represented in projects in a human readable format:

    ```json
    "Workspace": {
      "Terrain": {
        "$path": "path/to/terrain.rbxm"
        "$properties": {
          "MaterialColors": {
            "Grass": [10, 20, 30],
            "Asphalt": [40, 50, 60],
            "LeafyGrass": [255, 155, 55]
          }
        }
      }
    }
    ```

* Added better support for `Font` properties ([#731])

    `FontFace` properties may now be defined using implicit property syntax:

    ```json
    "TextBox": {
      "$className": "TextBox",
      "$properties": {
        "FontFace": {
          "family": "rbxasset://fonts/families/RobotoMono.json",
          "weight": "Thin",
          "style": "Normal"
        }
      }
    }
    ```

#### Patch visualizer and notifications

* Added a setting to control patch confirmation behavior ([#774])

    This is a new setting for controlling when the Rojo plugin prompts for confirmation before syncing. It has four options:
  * Initial (default): prompts only once for a project in a given Studio session
  * Always: always prompts for confirmation
  * Large Changes: only prompts when there are more than X changed instances. The number of instances is configurable - an additional setting for the number of instances becomes available when this option is chosen
  * Unlisted PlaceId: only prompts if the place ID is not present in servePlaceIds

* Added the ability to select Instances in patch visualizer ([#709])

    Double-clicking an instance in the patch visualizer sets Roblox Studio's selection to the instance.

* Added a sync reminder notification. ([#689])

    Rojo detects if you have previously synced to a place, and displays a notification reminding you to sync again:

    ![Rojo reminds you to sync a place that you've synced previously](https://user-images.githubusercontent.com/40185666/242397435-ccdfddf2-a63f-420c-bc18-a6e3d6455bba.png)

* Added rich Source diffs in patch visualizer ([#748])

    A "View Diff" button for script sources is now present in the patch visualizer. Clicking it displays a side-by-side diff of the script changes:

    ![The patch visualizer contains a "view diff" button](https://user-images.githubusercontent.com/40185666/256065992-3f03558f-84b0-45a1-80eb-901f348cf067.png)

    ![The "View Diff" button opens a widget that displays a diff](https://user-images.githubusercontent.com/40185666/256066084-1d9d8fe8-7dad-4ee7-a542-b4aee35a5644.png)

* Patch visualizer now indicates what changes failed to apply. ([#717])

    A clickable warning label is displayed when the Rojo plugin is unable to apply changes. Clicking the label displays precise information about which changes failed:

    ![Patch visualizer displays a clickable warning label when changes fail to apply](https://user-images.githubusercontent.com/40185666/252063660-f08399ef-1e16-4f1c-bed8-552821f98cef.png)

#### Miscellaneous

* Added `plugin` flag to the `build` command that outputs to the local plugins folder ([#735])

    This is a flag that builds a Rojo project into Roblox Studio's plugins directory. This allows you to build a Rojo project and load it into Studio as a plugin without having to type the full path to the plugins directory. It can be used like this: `rojo build <PATH-TO-PROJECT> --plugin <FILE-NAME>`

* Added new plugin template to the `init` command ([#738])

    This is a new template geared towards plugins. It is similar to the model template, but creates a `Script` instead of a `ModuleScript` in the `src` directory. It can be used like this: `rojo init --kind plugin`

* Added protection against syncing non-place projects as a place. ([#691])
* Add buttons for navigation on the Connected page ([#722])

### Fixes

* Significantly improved performance of `rojo sourcemap` ([#668])
* Fixed the diff visualizer of connected sessions. ([#674])
* Fixed disconnected session activity. ([#675])
* Skip confirming patches that contain only a datamodel name change. ([#688])
* Fix Rojo breaking when users undo/redo in Studio ([#708])
* Improve tooltip behavior ([#723])
* Better settings controls ([#725])
* Rework patch visualizer with many fixes and improvements ([#713], [#726], [#755])

[7.4.0-rc1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.0-rc1
[#633]: https://github.com/rojo-rbx/rojo/pull/633
[#668]: https://github.com/rojo-rbx/rojo/pull/668
[#674]: https://github.com/rojo-rbx/rojo/pull/674
[#675]: https://github.com/rojo-rbx/rojo/pull/675
[#688]: https://github.com/rojo-rbx/rojo/pull/688
[#689]: https://github.com/rojo-rbx/rojo/pull/689
[#691]: https://github.com/rojo-rbx/rojo/pull/691
[#708]: https://github.com/rojo-rbx/rojo/pull/708
[#709]: https://github.com/rojo-rbx/rojo/pull/709
[#713]: https://github.com/rojo-rbx/rojo/pull/713
[#717]: https://github.com/rojo-rbx/rojo/pull/717
[#722]: https://github.com/rojo-rbx/rojo/pull/722
[#723]: https://github.com/rojo-rbx/rojo/pull/723
[#725]: https://github.com/rojo-rbx/rojo/pull/725
[#726]: https://github.com/rojo-rbx/rojo/pull/726
[#731]: https://github.com/rojo-rbx/rojo/pull/731
[#735]: https://github.com/rojo-rbx/rojo/pull/735
[#738]: https://github.com/rojo-rbx/rojo/pull/738
[#748]: https://github.com/rojo-rbx/rojo/pull/748
[#755]: https://github.com/rojo-rbx/rojo/pull/755
[#765]: https://github.com/rojo-rbx/rojo/pull/765
[#770]: https://github.com/rojo-rbx/rojo/pull/770
[#771]: https://github.com/rojo-rbx/rojo/pull/771
[#774]: https://github.com/rojo-rbx/rojo/pull/774
[rbx-dom#296]: https://github.com/rojo-rbx/rbx-dom/pull/296
[rbx-dom#299]: https://github.com/rojo-rbx/rbx-dom/pull/299

## [7.3.0] (April 22, 2023)

* Added `$attributes` to project format. ([#574])
* Added `--watch` flag to `rojo sourcemap`. ([#602])
* Added support for `init.csv` files. ([#594])
* Added real-time sync status to the Studio plugin. ([#569])
* Added support for copying error messages to the clipboard. ([#614])
* Added sync locking for Team Create. ([#590])
* Added support for specifying HTTP or HTTPS protocol in plugin. ([#642])
* Added tooltips to buttons in the Studio plugin. ([#637])
* Added visual diffs when connecting from the Studio plugin. ([#603])
* Host and port are now saved in the Studio plugin. ([#613])
* Improved padding on notifications in Studio plugin. ([#589])
* Renamed `Common` to `Shared` in the default Rojo project. ([#611])
* Reduced the minimum size of the Studio plugin widget. ([#606])
* Fixed current directory in `rojo fmt-project`. ([#581])
* Fixed errors after a session has already ended. ([#587])
* Fixed an uncommon security permission error ([#619])

[7.3.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.3.0
[#569]: https://github.com/rojo-rbx/rojo/pull/569
[#574]: https://github.com/rojo-rbx/rojo/pull/574
[#581]: https://github.com/rojo-rbx/rojo/pull/581
[#587]: https://github.com/rojo-rbx/rojo/pull/587
[#589]: https://github.com/rojo-rbx/rojo/pull/589
[#590]: https://github.com/rojo-rbx/rojo/pull/590
[#594]: https://github.com/rojo-rbx/rojo/pull/594
[#602]: https://github.com/rojo-rbx/rojo/pull/602
[#603]: https://github.com/rojo-rbx/rojo/pull/603
[#606]: https://github.com/rojo-rbx/rojo/pull/606
[#611]: https://github.com/rojo-rbx/rojo/pull/611
[#613]: https://github.com/rojo-rbx/rojo/pull/613
[#614]: https://github.com/rojo-rbx/rojo/pull/614
[#619]: https://github.com/rojo-rbx/rojo/pull/619
[#637]: https://github.com/rojo-rbx/rojo/pull/637
[#642]: https://github.com/rojo-rbx/rojo/pull/642

## [7.2.1] (July 8, 2022)

* Fixed notification sound by changing it to a generic sound. ([#566])
* Added setting to turn off sound effects. ([#568])

[7.2.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.2.1
[#566]: https://github.com/rojo-rbx/rojo/pull/566
[#568]: https://github.com/rojo-rbx/rojo/pull/568

## [7.2.0] (June 29, 2022)

* Added support for `.luau` files. ([#552])
* Added support for live syncing Attributes and Tags. ([#553])
* Added notification popups in the Roblox Studio plugin. ([#540])
* Fixed `init.meta.json` when used with `init.lua` and related files. ([#549])
* Fixed incorrect output when serving from a non-default address or port ([#556])
* Fixed Linux binaries not running on systems with older glibc. ([#561])
* Added `camelCase` casing for JSON models, deprecating `PascalCase` names. ([#563])
* Switched from structopt to clap for command line argument parsing.
* Significantly improved performance of building and serving. ([#548])
* Increased minimum supported Rust version to 1.57.0. ([#564])

[7.2.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.2.0
[#540]: https://github.com/rojo-rbx/rojo/pull/540
[#548]: https://github.com/rojo-rbx/rojo/pull/548
[#549]: https://github.com/rojo-rbx/rojo/pull/549
[#552]: https://github.com/rojo-rbx/rojo/pull/552
[#553]: https://github.com/rojo-rbx/rojo/pull/553
[#556]: https://github.com/rojo-rbx/rojo/pull/556
[#561]: https://github.com/rojo-rbx/rojo/pull/561
[#563]: https://github.com/rojo-rbx/rojo/pull/563
[#564]: https://github.com/rojo-rbx/rojo/pull/564

## [7.1.1] (May 26, 2022)

* Fixed sourcemap command not stripping paths correctly ([#544])
* Fixed Studio plugin settings not saving correctly.

[7.1.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.1.1
[#544]: https://github.com/rojo-rbx/rojo/pull/544

## [7.1.0] (May 22, 2022)

* Added support for specifying an address to be used by default in project files. ([#507])
* Added support for optional paths in project files. ([#472])
* Added support for the new Open Cloud API when uploading. ([#504])
* Added `sourcemap` command for generating sourcemaps to feed into other tools. ([#530])
* Added PluginActions for connecting/disconnecting a session ([#537])
* Added changing toolbar icon to indicate state ([#538])

[7.1.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.1.0
[#472]: https://github.com/rojo-rbx/rojo/pull/472
[#504]: https://github.com/rojo-rbx/rojo/pull/504
[#507]: https://github.com/rojo-rbx/rojo/pull/507
[#530]: https://github.com/rojo-rbx/rojo/pull/530
[#537]: https://github.com/rojo-rbx/rojo/pull/537
[#538]: https://github.com/rojo-rbx/rojo/pull/538

## [7.0.0] (December 10, 2021)

* Fixed Rojo's interactions with properties enabled by FFlags that are not yet enabled. ([#493])
* Improved output in Roblox Studio plugin when bad property data is encountered.
* Reintroduced support for CFrame shorthand syntax in Rojo project and `.meta.json` files, matching Rojo 6. ([#430])
* Connection settings are now remembered when reconnecting in Roblox Studio. ([#500])
* Updated reflection database to Roblox v503.

[7.0.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0
[#430]: https://github.com/rojo-rbx/rojo/issues/430
[#493]: https://github.com/rojo-rbx/rojo/pull/493
[#500]: https://github.com/rojo-rbx/rojo/pull/500

## [7.0.0-rc.3] (October 19, 2021)

This is the last release candidate for Rojo 7. In an effort to get Rojo 7 out the door, we'll be freezing features from here on out, something we should've done a couple months ago.

Expect to see Rojo 7 stable soon!

* Added support for writing `Tags` in project files, model files, and meta files. ([#484])
* Adjusted Studio plugin colors to match Roblox Studio palette. ([#482])
* Improved experimental two-way sync feature by batching changes. ([#478])

[7.0.0-rc.3]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0-rc.3
[#478]: https://github.com/rojo-rbx/rojo/pull/478
[#482]: https://github.com/rojo-rbx/rojo/pull/482
[#484]: https://github.com/rojo-rbx/rojo/pull/484

## 7.0.0-rc.2 (October 19, 2021)

(Botched release due to Git mishap, oops!)

## [7.0.0-rc.1] (August 23, 2021)

In Rojo 6 and previous Rojo 7 alphas, an explicit Vector3 property would be written like this:

```json
{
    "className": "Part",
    "properties": {
        "Position": {
            "Type": "Vector3",
            "Value": [1, 2, 3]
        }
    }
}
```

For Rojo 7, this will need to be changed to:

```json
{
    "className": "Part",
    "properties": {
        "Position": {
            "Vector3": [1, 2, 3]
        }
    }
}
```

The shorthand property format that most users use is not impacted. For reference, it looks like this:

```json
{
    "className": "Part",
    "properties": {
        "Position": [1, 2, 3]
    }
}
```

* Major breaking change: changed property syntax for project files; shorthand syntax is unchanged.
* Added the `fmt-project` subcommand for formatting Rojo project files.
* Improved error output for many subcommands.
* Updated to stable versions of rbx-dom libraries.
* Updated async infrastructure, which should fix a handful of bugs. ([#459])
* Fixed syncing refs in the Roblox Studio plugin ([#462], [#466])
* Added support for long paths on Windows. ([#464])

[7.0.0-rc.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0-rc.1
[#459]: https://github.com/rojo-rbx/rojo/pull/459
[#462]: https://github.com/rojo-rbx/rojo/pull/462
[#464]: https://github.com/rojo-rbx/rojo/pull/464
[#466]: https://github.com/rojo-rbx/rojo/pull/466

## [7.0.0-alpha.4] (May 5, 2021)

* Added the `gameId` and `placeId` optional properties to project files.

  * When connecting from the Rojo Roblox Studio plugin, Rojo will set the game and place ID of the current place to these values, if set.
  * This is equivalent to running `game:SetUniverseId(...)` and `game:SetPlaceId(...)` from the command bar in Studio.
* Added "EXPERIMENTAL!" label to two-way sync toggle in Rojo's Roblox Studio plugin.
* Fixed `Name` and `Parent` properties being allowed in Rojo projects. ([#413])
* Fixed "Open Scripts Externally" feature crashing Studio. ([#369])
* Empty `.model.json` files will no longer cause errors. ([#420])
* When specifying `$path` on a service, Rojo now keeps the correct class name. ([#331])
* Improved error messages for misconfigured projects.

[7.0.0-alpha.4]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0-alpha.4
[#331]: https://github.com/rojo-rbx/rojo/issues/331
[#369]: https://github.com/rojo-rbx/rojo/issues/369
[#413]: https://github.com/rojo-rbx/rojo/pull/413
[#420]: https://github.com/rojo-rbx/rojo/pull/420

## [7.0.0-alpha.3] (February 19, 2021)

* Updated dependencies, fixing `OptionalCoordinateFrame`-related issues.
* Added `--address` flag to `rojo serve` to allow for external connections. ([#403])

[7.0.0-alpha.3]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0-alpha.3
[#403]: https://github.com/rojo-rbx/rojo/pull/403

## [7.0.0-alpha.2] (February 19, 2021)

* Fixed incorrect protocol version between the client and server.

[7.0.0-alpha.2]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0-alpha.2

## [7.0.0-alpha.1] (February 18, 2021)

This release includes a brand new implementation of the Roblox DOM. It brings performance improvements, much better support for `rbxl` and `rbxm` files, and a better internal API.

* Added support for all remaining property types.
* Added support for the entire Roblox binary model format.
* Changed `rojo upload` to upload binary places and models instead of XML.
  * This should make using `rojo upload` much more feasible for large places.
* **Breaking**: Changed format of some types of values in `project.json`, `model.json`, and `meta.json` files.
  * This should impact few projects. See [this file][allValues.json] for new examples of each property type.

Formatting of types will change more before the stable release of Rojo 7. We're hoping to use this opportunity to normalize some of the case inconsistency introduced in Rojo 0.5.

[7.0.0-alpha.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0-alpha.1
[allValues.json]: https://github.com/rojo-rbx/rojo/blob/f4a790eb50b74e482000bad1dcfe22533992fb20/plugin/rbx_dom_lua/src/allValues.json

## [6.0.2] (February 9, 2021)

* Fixed `rojo upload` to handle CSRF challenges.

[6.0.2]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.2

## [6.0.1] (January 22, 2021)

* Fixed `rojo upload` requests being rejected by Roblox

[6.0.1]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.1

## [6.0.0] (January 16, 2021)

* Improved server error messages
  * The server will now keep running in more error cases
* Fixed Rojo being unable to diff ClassName changes

[6.0.0]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.0

## [6.0.0-rc.4] (December 14, 2020)

* Added brand new Rojo UI ([#367])
* Added `projectName` to `/api/rojo` output.

[6.0.0-rc.4]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.0-rc.4
[#367]: https://github.com/rojo-rbx/rojo/pull/367

## [6.0.0-rc.3] (November 19, 2020)

* Fixed the Rojo plugin attempting to write the non-scriptable properties `Instance.SourceAssetId` and `HttpServer.HttpEnabled`.
* Fixed the Rojo plugin's handling of null referents.

[6.0.0-rc.3]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.0-rc.3

## [6.0.0-rc.2] (November 19, 2020)

* Fixed crash when malformed CSV files are put into a project. ([#310])
* Fixed incorrect string escaping when producing Lua code from JSON files. ([#314])
* Fixed performance issues introduced in Rojo 6.0.0-rc.1. ([#317])
* Fixed `rojo plugin install` subcommand failing for everyone except Rojo developers. ([#320])
* Updated default place template to take advantage of [#210].
* Enabled glob ignore patterns by default and removed the `unstable_glob_ignore` feature.
  * `globIgnorePaths` can be set on a project to a list of globs to ignore.
* The Rojo plugin now completes as much as it can from a patch without disconnecting. Warnings are shown in the console.
* Fixed 6.0.0-rc.1 regression causing instances that changed ClassName to instead... not change ClassName.

[6.0.0-rc.2]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.0-rc.2
[#210]: https://github.com/rojo-rbx/rojo/pull/210
[#310]: https://github.com/rojo-rbx/rojo/issues/310
[#314]: https://github.com/rojo-rbx/rojo/issues/314
[#317]: https://github.com/rojo-rbx/rojo/issues/317
[#320]: https://github.com/rojo-rbx/rojo/issues/320

## [6.0.0-rc.1] (March 29, 2020)

This release jumped from 0.6.0 to 6.0.0. Rojo has been in use in production for many users for quite a long times, and so 6.0 is a more accurate reflection of Rojo's version than a pre-1.0 version.

* Added basic settings panel to plugin, with two settings:
  * "Open Scripts Externally": When enabled, opening a script in Studio will instead open it in your default text editor.
  * "Two-Way Sync": When enabled, Rojo will attempt to save changes to your place back to the filesystem. **Very early feature, very broken, beware!**
* Added `--color` option to force-enable or force-disable color in Rojo's output.
* Added support for turning `.json` files into `ModuleScript` instances ([#308])
* Added `rojo plugin install` and `rojo plugin uninstall` to allow Rojo to manage its Roblox Studio plugin. ([#304])
* Class names no longer need to be specified for Roblox services in Rojo projects. ([#210])
* The server half of **experimental** two-way sync is now enabled by default.
* Increased default logging verbosity in commands like `rojo build`.
* Rojo now requires a project file again, just like 0.5.4.

[6.0.0-rc.1]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.0-rc.1
[#304]: https://github.com/rojo-rbx/rojo/pull/304
[#308]: https://github.com/rojo-rbx/rojo/pull/308

## [0.6.0-alpha.3] (March 13, 2020)

* Added `--watch` argument to `rojo build`. ([#284])
* Added dark theme support to plugin. ([#241])
* Added a revamped `rojo init` command, which will now create more complete projects.
* Added the `rojo doc` command, which opens Rojo's documentation in your browser.
* Fixed many crashes from malformed projects and filesystem edge cases in `rojo serve`.
* Simplified filesystem access code dramatically.
* Improved error reporting and logging across the board.
  * Log messages have a less noisy prefix.
  * Any thread panicking now causes Rojo to abort instead of existing as a zombie.
  * Errors now have a list of causes, helping make many errors more clear.

[0.6.0-alpha.3]: https://github.com/rojo-rbx/rojo/releases/tag/v0.6.0-alpha.3
[#241]: https://github.com/rojo-rbx/rojo/issues/241
[#284]: https://github.com/rojo-rbx/rojo/pull/284

## [0.6.0-alpha.2] (March 6, 2020)

* Fixed `rojo upload` command always uploading models.
* Removed `--kind` parameter to `rojo upload`; Rojo now automatically uploads the correct kind of asset based on your project file.

[0.6.0-alpha.2]: https://github.com/rojo-rbx/rojo/releases/tag/v0.6.0-alpha.2

## [0.5.4] (February 26, 2020)

This is a general maintenance release for the Rojo 0.5.x release series.

* Updated reflection database and other dependencies.
* First stable release with binaries for macOS and Linux.

[0.5.4]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.4

## [0.6.0-alpha.1] (January 22, 2020)

### General

* Added support for nested project files. ([#95])
* Added project file hot-reloading. ([#10])
* Fixed Rojo dropping Ref properties ([#142])
  * This means that properties like `PrimaryPart` now work!
* Improved live sync protocol to reduce round-trips and improve syncing consistency.
* Improved support for binary model files and places.

### Command Line

* Added `--verbose`/`-v` flag, which can be specified multiple times to increase verbosity.
* Added support for automatically finding Roblox Studio's auth cookie for `rojo upload` on Windows.
* Added support for building, serving and uploading sources that aren't Rojo projects.
* Improved feedback from `rojo serve`.
* Removed support for legacy `roblox-project.json` projects, deprecated in an early Rojo 0.5.0 alpha.
* Rojo no longer traverses directories upwards looking for project files.
  * Though undocumented, Rojo 0.5.x will search for a project file contained in any ancestor folders. This feature was removed to better support other 0.6.x features.

### Roblox Studio Plugin

* Added "connecting" state to improve experience when live syncing.
* Added "error" state to show errors in a place that isn't the output panel.
* Improved diagnostics for when the Rojo plugin cannot create an instance.

[0.6.0-alpha.1]: https://github.com/rojo-rbx/rojo/releases/tag/v0.6.0-alpha.1
[#10]: https://github.com/rojo-rbx/rojo/issues/10
[#95]: https://github.com/rojo-rbx/rojo/issues/95
[#142]: https://github.com/rojo-rbx/rojo/issues/142

## [0.5.3] (October 15, 2019)

* Fixed an issue where Rojo would throw an error when encountering recently-added instance classes.

[0.5.3]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.3

## [0.5.2] (October 14, 2019)

* Fixed an issue where `LocalizationTable` instances would have their column order randomized. ([#173])

[0.5.2]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.2
[#173]: https://github.com/rojo-rbx/rojo/issues/173

## [0.5.1] (October 4, 2019)

* Fixed an issue where Rojo would drop changes if they happened too quickly ([#252])
* Improved diagnostics for when the Rojo plugin cannot create an instance.
* Updated dependencies
  * This brings Rojo's reflection database from client release 395 to client release 404.

[0.5.1]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.1
[#252]: https://github.com/rojo-rbx/rojo/issues/252

## [0.5.0] (August 27, 2019)

* Changed `.model.json` naming, which may require projects to migrate ambiguous cases:
  * The file name now takes precedence over the `Name` field in the model, like Rojo 0.4.x.
  * The `Name` field of the top-level instance is now optional. It's recommended that you remove it from your models.
  * Rojo will emit a warning when `Name` is specified and does not match the name from the file.
* Fixed `Rect` values being set to `0, 0, 0, 0` when synced with the Rojo plugin. ([#201])
* Fixed live-syncing of `PhysicalProperties`, `NumberSequence`, and `ColorSequence` values

[0.5.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0
[#201]: https://github.com/rojo-rbx/rojo/issues/201

## [0.5.0-alpha.13] (August 2, 2019)

* Bumped minimum Rust version to 1.34.0.
* Fixed default port documentation in `rojo serve --help` ([#219])
* Fixed BrickColor support by upgrading Roblox-related dependencies

[0.5.0-alpha.13]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.13
[#219]: https://github.com/rojo-rbx/rojo/issues/219

## [0.5.0-alpha.12] (July 2, 2019)

* Added `.meta.json` files
  * `init.meta.json` files replace `init.model.json` files from Rojo 0.4.x ([#183])
  * Other `.meta.json` files allow attaching extra data to other files ([#189])
* Added support for infinite and NaN values in types like `Vector2` when building models and places.
  * These types aren't supported for live-syncing yet due to limitations around JSON encoding.
* Added support for using `SharedString` values when building XML models and places.
* Added support for live-syncing `CollectionService` tags.
* Added a warning when building binary place files, since they're still experimental and have bugs.
* Added a warning when trying to use Rojo 0.5.x with a Rojo 0.4.x-only project.
* Added a warning when a Rojo project contains keys that start with `$`, which are reserved names. ([#191])
* Rojo now throws an error if unknown keys are found most files.
* Added an icon to the plugin's toolbar button
* Changed the plugin to use a docking widget for all UI.
* Changed the plugin to ignore unknown properties when live-syncing.
  * Rojo's approach to this problem might change later, like with a strict model mode ([#190]) or another approach.
* Upgraded to reflection database from client release 388.
* Updated Rojo's branding to shift the color palette to make it work better on dark backgrounds

[0.5.0-alpha.12]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.12
[#183]: https://github.com/rojo-rbx/rojo/pull/183
[#189]: https://github.com/rojo-rbx/rojo/pull/189
[#190]: https://github.com/rojo-rbx/rojo/issues/190
[#191]: https://github.com/rojo-rbx/rojo/issues/191

## [0.5.0-alpha.11] (May 29, 2019)

* Added support for implicit property values in JSON model files ([#154])
* `Content` properties can now be specified in projects and model files as regular string literals.
* Added support for `BrickColor` properties.
* Added support for properties added in client release 384, like `Lighting.Technology` being set to `"ShadowMap"`.
* Improved performance when working with XML models and places
* Fixed serializing empty `Content` properties as XML
* Fixed serializing infinite and NaN floating point properties in XML
* Improved compatibility with XML models
* Plugin should now be able to live-sync more properties, and ignore ones it can't, like `Lighting.Technology`.

[0.5.0-alpha.11]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.11
[#154]: https://github.com/rojo-rbx/rojo/pull/154

## 0.5.0-alpha.10

* This release was a dud due to [#176] and was rolled back.

[#176]: https://github.com/rojo-rbx/rojo/issues/176

## [0.5.0-alpha.9] (April 4, 2019)

* Changed `rojo build` to use buffered I/O, which can make it up to 2x faster in some cases.
  * Building [*Road Not Taken*](https://github.com/LPGhatguy/roads) to an `rbxlx` file dropped from 150ms to 70ms on my machine
* Fixed `LocalizationTable` instances being made from `csv` files incorrectly interpreting empty rows and columns. ([#149])
* Fixed CSV files with entries that parse as numbers causing Rojo to panic. ([#152])
* Improved error messages when malformed CSV files are found in a Rojo project.

[0.5.0-alpha.9]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.9
[#149]: https://github.com/rojo-rbx/rojo/pull/149
[#152]: https://github.com/rojo-rbx/rojo/pull/152

## [0.5.0-alpha.8] (March 29, 2019)

* Added support for a bunch of new types when dealing with XML model/place files:
  * `ColorSequence`
  * `Float64`
  * `Int64`
  * `NumberRange`
  * `NumberSequence`
  * `PhysicalProperties`
  * `Ray`
  * `Rect`
  * `Ref`
* Improved server instance ordering behavior when files are added during a live session ([#135])
* Fixed error being thrown when trying to unload the Rojo plugin.
* Added partial fix for [#141] for `Lighting.Technology`, which should restore live sync functionality for the default project file.

[0.5.0-alpha.8]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.8
[#135]: https://github.com/rojo-rbx/rojo/pull/135
[#141]: https://github.com/rojo-rbx/rojo/issues/141

## [0.5.0-alpha.6] (March 19, 2019)

* Fixed `rojo init` giving unexpected results by upgrading to `rbx_dom_weak` 1.1.0
* Fixed live server not responding when the Rojo plugin is connected ([#133])
* Updated default place file:
  * Improved default properties to be closer to Studio's built-in 'Baseplate' template
  * Added a baseplate to the project file (Thanks, [@AmaranthineCodices](https://github.com/AmaranthineCodices/)!)
* Added more type support to Rojo plugin
* Fixed some cases where the Rojo plugin would leave around objects that it knows should be deleted
* Updated plugin to correctly listen to `Plugin.Unloading` when installing or uninstalling new plugins

[0.5.0-alpha.6]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.6
[#133]: https://github.com/rojo-rbx/rojo/issues/133

## [0.5.0-alpha.5] (March 1, 2019)

* Upgraded core dependencies, which improves compatibility for lots of instance types
  * Upgraded from `rbx_tree` 0.2.0 to `rbx_dom_weak` 1.0.0
  * Upgraded from `rbx_xml` 0.2.0 to `rbx_xml` 0.4.0
  * Upgraded from `rbx_binary` 0.2.0 to `rbx_binary` 0.4.0
* Added support for non-primitive types in the Rojo plugin.
  * Types like `Color3` and `CFrame` can now be updated live!
* Fixed plugin assets flashing in on first load ([#121])
* Changed Rojo's HTTP server from Rouille to Hyper, which reduced the release size by around a megabyte.
* Added property type inference to projects, which makes specifying services a lot easier ([#130])
* Made error messages from invalid and missing files more user-friendly

[0.5.0-alpha.5]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.5
[#121]: https://github.com/rojo-rbx/rojo/issues/121
[#130]: https://github.com/rojo-rbx/rojo/pull/130

## [0.5.0-alpha.4] (February 8, 2019)

* Added support for nested partitions ([#102])
* Added support for 'transmuting' partitions ([#112])
* Added support for aliasing filesystem paths ([#105])
* Changed Windows builds to statically link the CRT ([#89])

[0.5.0-alpha.4]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.4
[#89]: https://github.com/rojo-rbx/rojo/issues/89
[#102]: https://github.com/rojo-rbx/rojo/issues/102
[#105]: https://github.com/rojo-rbx/rojo/issues/105
[#112]: https://github.com/rojo-rbx/rojo/issues/112

## [0.5.0-alpha.3] (February 1, 2019)

* Changed default project file name from `roblox-project.json` to `default.project.json` ([#120])
  * The old file name will still be supported until 0.5.0 is fully released.
* Added warning when loading project files that don't end in `.project.json`
  * This new extension enables Rojo to distinguish project files from random JSON files, which is necessary to support nested projects.
* Added new (empty) diagnostic page served from the server
* Added better error messages for when a file is missing that's referenced by a Rojo project
* Added support for visualization endpoints returning GraphViz source when Dot is not available
* Fixed an in-memory filesystem regression introduced recently ([#119])

[0.5.0-alpha.3]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.3
[#119]: https://github.com/rojo-rbx/rojo/pull/119
[#120]: https://github.com/rojo-rbx/rojo/pull/120

## [0.5.0-alpha.2] (January 28, 2019)

* Added support for `.model.json` files, compatible with 0.4.x
* Fixed in-memory filesystem not handling out-of-order filesystem change events
* Fixed long-polling error caused by a promise mixup ([#110])

[0.5.0-alpha.2]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.2
[#110]: https://github.com/rojo-rbx/rojo/issues/110

## [0.5.0-alpha.1] (January 25, 2019)

* Changed plugin UI to be way prettier
  * Thanks to [Reselim](https://github.com/Reselim) for the design!
* Changed plugin error messages to be a little more useful
* Removed unused 'Config' button in plugin UI
* Fixed bug where bad server responses could cause the plugin to be in a bad state
* Upgraded to rbx\_tree, rbx\_xml, and rbx\_binary 0.2.0, which dramatically expands the kinds of properties that Rojo can handle, especially in XML.

[0.5.0-alpha.1]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.1

## [0.5.0-alpha.0] (January 14, 2019)

* "Epiphany" rewrite, in progress since the beginning of time
* New live sync protocol
  * Uses HTTP long polling to reduce request count and improve responsiveness
* New project format
  * Hierarchical, preventing overlapping partitions
* Added `rojo build` command
  * Generates `rbxm`, `rbxmx`, `rbxl`, or `rbxlx` files out of your project
  * Usage: `rojo build <PROJECT> --output <OUTPUT>.rbxm`
* Added `rojo upload` command
  * Generates and uploads a place or model to roblox.com out of your project
  * Usage: `rojo upload <PROJECT> --cookie "<ROBLOSECURITY>" --asset_id <PLACE_ID>`
* New plugin
  * Only one button now, "Connect"
  * New UI to pick server address and port
  * Better error reporting
* Added support for `.csv` files turning into `LocalizationTable` instances
* Added support for `.txt` files turning into `StringValue` instances
* Added debug visualization code to diagnose problems
  * `/visualize/rbx` and `/visualize/imfs` show instance and file state respectively; they require GraphViz to be installed on your machine.
* Added optional place ID restrictions to project files
  * This helps prevent syncing in content to the wrong place
  * Multiple places can be specified, like when building a multi-place game
* Added support for specifying properties on services in project files

[0.5.0-alpha.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.0

## [0.4.13] (November 12, 2018)

* When `rojo.json` points to a file or directory that does not exist, Rojo now issues a warning instead of throwing an error and exiting

[0.4.13]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.13

## [0.4.12] (June 21, 2018)

* Fixed obscure assertion failure when renaming or deleting files ([#78])
* Added a `PluginAction` for the sync in command, which should help with some automation scripts ([#80])

[0.4.12]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.12
[#78]: https://github.com/rojo-rbx/rojo/issues/78
[#80]: https://github.com/rojo-rbx/rojo/pull/80

## [0.4.11] (June 10, 2018)

* Defensively insert existing instances into RouteMap; should fix most duplication cases when syncing into existing trees.
* Fixed incorrect synchronization from `Plugin:_pull` that would cause polling to create issues
* Fixed incorrect file routes being assigned to `init.lua` and `init.model.json` files
* Untangled route handling-internals slightly

[0.4.11]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.11

## [0.4.10] (June 2, 2018)

* Added support for `init.model.json` files, which enable versioning `Tool` instances (among other things) with Rojo. ([#66])
* Fixed obscure error when syncing into an invalid service.
* Fixed multiple sync processes occurring when a server ID mismatch is detected.

[0.4.10]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.10
[#66]: https://github.com/rojo-rbx/rojo/issues/66

## [0.4.9] (May 26, 2018)

* Fixed warning when renaming or removing files that would sometimes corrupt the instance cache ([#72])
* JSON models are no longer as strict -- `Children` and `Properties` are now optional.

[0.4.9]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.9
[#72]: https://github.com/rojo-rbx/rojo/pull/72

## [0.4.8] (May 26, 2018)

* Hotfix to prevent errors from being thrown when objects managed by Rojo are deleted

[0.4.8]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.8

## [0.4.7] (May 25, 2018)

* Added icons to the Rojo plugin, made by [@Vorlias](https://github.com/Vorlias)! ([#70])
* Server will now issue a warning if no partitions are specified in `rojo serve` ([#40])

[0.4.7]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.7
[#40]: https://github.com/rojo-rbx/rojo/issues/40
[#70]: https://github.com/rojo-rbx/rojo/pull/70

## [0.4.6] (May 21, 2018)

* Rojo handles being restarted by Roblox Studio more gracefully ([#67])
* Folders should no longer get collapsed when syncing occurs.
* **Significant** robustness improvements with regards to caching.
  * **This should catch all existing script duplication bugs.**
  * If there are any bugs with script duplication or caching in the future, restarting the Rojo server process will fix them for that session.
* Fixed message in plugin not being prefixed with `Rojo:`.

[0.4.6]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.6
[#67]: https://github.com/rojo-rbx/rojo/issues/67

## [0.4.5] (May 1, 2018)

* Rojo messages are now prefixed with `Rojo:` to make them stand out in the output more.
* Fixed server to notice file changes *much* more quickly. (200ms vs 1000ms)
* Server now lists name of project when starting up.
* Rojo now throws an error if no project file is found. ([#63])
* Fixed multiple sync operations occuring at the same time. ([#61])
* Partitions targeting files directly now work as expected. ([#57])

[0.4.5]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.5
[#57]: https://github.com/rojo-rbx/rojo/issues/57
[#61]: https://github.com/rojo-rbx/rojo/issues/61
[#63]: https://github.com/rojo-rbx/rojo/issues/63

## [0.4.4] (April 7, 2018)

* Fix small regression introduced in 0.4.3

[0.4.4]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.4

## [0.4.3] (April 7, 2018)

* Plugin now automatically selects `HttpService` if it determines that HTTP isn't enabled ([#58])
* Plugin now has much more robust handling and will wipe all state when the server changes.
  * This should fix issues that would otherwise be solved by restarting Roblox Studio.

[0.4.3]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.3
[#58]: https://github.com/rojo-rbx/rojo/pull/58

## [0.4.2] (April 4, 2018)

* Fixed final case of duplicated instance insertion, caused by reconciled instances not being inserted into `RouteMap`.
  * The reconciler is still not a perfect solution, especially if script instances get moved around without being destroyed. I don't think this can be fixed before a big refactor.

[0.4.2]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.2

## [0.4.1] (April 1, 2018)

* Merged plugin repository into main Rojo repository for easier tracking.
* Improved `RouteMap` object tracking; this should fix some cases of duplicated instances being synced into the tree.

[0.4.1]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.1

## [0.4.0] (March 27, 2018)

* Protocol version 1, which shifts more responsibility onto the server
  * This is a **major breaking** change!
  * The server now has a content of 'filter plugins', which transform data at various stages in the pipeline
  * The server now exposes Roblox instance objects instead of file contents, which lines up with how `rojo pack` will work, and paves the way for more robust syncing.
* Added `*.model.json` files, which let you embed small Roblox objects into your Rojo tree.
* Improved error messages in some cases ([#46])

[0.4.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.0
[#46]: https://github.com/rojo-rbx/rojo/issues/46

## [0.3.2] (December 20, 2017)

* Fixed `rojo serve` failing to correctly construct an absolute root path when passed as an argument
* Fixed intense CPU usage when running `rojo serve`

[0.3.2]: https://github.com/rojo-rbx/rojo/releases/tag/v0.3.2

## [0.3.1] (December 14, 2017)

* Improved error reporting when invalid JSON is found in a `rojo.json` project
  * These messages are passed on from Serde

[0.3.1]: https://github.com/rojo-rbx/rojo/releases/tag/v0.3.1

## [0.3.0] (December 12, 2017)

* Factored out the plugin into a separate repository
* Fixed server when using a file as a partition
  * Previously, trailing slashes were put on the end of a partition even if the read request was an empty string. This broke file reading on Windows when a partition pointed to a file instead of a directory!
* Started running automatic tests on Travis CI ([#9])

[0.3.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.3.0
[#9]: https://github.com/rojo-rbx/rojo/pull/9

## [0.2.3] (December 4, 2017)

* Plugin only release
* Tightened `init` file rules to only match script files
  * Previously, Rojo would sometimes pick up the wrong file when syncing

[0.2.3]: https://github.com/rojo-rbx/rojo/releases/tag/v0.2.3

## [0.2.2] (December 1, 2017)

* Plugin only release
* Fixed broken reconciliation behavior with `init` files

[0.2.2]: https://github.com/rojo-rbx/rojo/releases/tag/v0.2.2

## [0.2.1] (December 1, 2017)

* Plugin only release
* Changes default port to 8000

[0.2.1]: https://github.com/rojo-rbx/rojo/releases/tag/v0.2.1

## [0.2.0] (December 1, 2017)

* Support for `init.lua` like rbxfs and rbxpacker
* More robust syncing with a new reconciler

[0.2.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.2.0

## [0.1.0] (November 29, 2017)

* Initial release, functionally very similar to [rbxfs](https://github.com/LPGhatguy/rbxfs)

[0.1.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.1.0

[#1115]: https://github.com/rojo-rbx/rojo/issues/1115

[#1204]: https://github.com/rojo-rbx/rojo/issues/1204
[#1229]: https://github.com/rojo-rbx/rojo/issues/1229

[#1007]: https://github.com/rojo-rbx/rojo/issues/1007
[#287]: https://github.com/rojo-rbx/rojo/issues/287

[#1263]: https://github.com/rojo-rbx/rojo/issues/1263
[#1141]: https://github.com/rojo-rbx/rojo/issues/1141
[#926]: https://github.com/rojo-rbx/rojo/issues/926

[#895]: https://github.com/rojo-rbx/rojo/issues/895

[#1279]: https://github.com/rojo-rbx/rojo/issues/1279
