# Fully managed Rojo workflow

This guide targets the Windows Rojo 8.0.0 fork, not an upstream release or a
VSCode extension. Start with [local setup](windows-local-install.md), then use
the [runnable example](../examples/agent-workflow/default.project.json).
[Cookbook](rojo-cookbook.md) and [coding-agent guide](coding-agent-guide.md)
provide task-sized recipes.

## Source layout

"Fully managed" means that the intended scripts, hierarchy and saved models
have reviewed file representations. Studio remains the editor/preview, not an
automatic exporter of every property change.

| Example source | Result |
|---|---|
| `src/shared/Greeting.luau` | `ReplicatedStorage.Shared.Greeting`, a ModuleScript |
| `src/shared/Greeting.meta.json` | Attributes of that same ModuleScript |
| `src/server/Boot.server.luau` | `ServerScriptService.Game.Boot`, Script with server RunContext |
| `src/client/Boot.client.luau` | `StarterPlayer.StarterPlayerScripts.Client.Boot`, Script with client RunContext |
| `assets/Demo.model.json` | `Workspace.Demo`, Model with an anchored Pad and a PrimaryPart reference |
| `default.project.json` | Typed services, both StarterPlayer containers and source mappings |

The example opts into `$version: 2`, so client/server suffixes produce modern
Script RunContexts and unknown Studio descendants are preserved by default.
That preservation does not import unknown objects into source control. An
unversioned/V1 project retains its legacy defaults. Use explicit project policy
and inspect initial diffs when changing versions or deletion behavior.

## Build and inspect

From this repository in PowerShell7:

```powershell
$Repo = (Get-Location).Path
$Rojo = (Resolve-Path -LiteralPath "$Repo\target\debug\rojo.exe").Path
$Project = "$Repo\examples\agent-workflow\default.project.json"
$Out = "$Repo\target\agent-workflow"
New-Item -ItemType Directory -Path $Out -Force | Out-Null
& $Rojo build $Project -o "$Out\game.rbxlx"
& $Rojo build $Project -o "$Out\game.rbxl"
& $Rojo sourcemap $Project --include-non-scripts --relative-to-output -o "$Out\sourcemap.json"
```

Keep output outside `src`/`assets`. The XML file is inspectable; the binary is
the normal Studio place format. The sourcemap lists effective paths/classes and
modern RunContext. `--relative-to-output` resolves its paths against the output
directory, not whichever directory a later tool happens to use.

Open the built place in Studio. Confirm Shared.Greeting, both Boot scripts,
both StarterPlayer containers, the Pad size12/1/12 and Demo.PrimaryPart = Pad.
Building parses source representations; it does not execute gameplay scripts.

## Connect for file edits

Install the plugin from the same local build as described in the setup guide.
Then keep this server terminal running:

```powershell
& $Rojo serve $Project --address 127.0.0.1 --port 34872 --sync-error-log --sync-warn-log
```

Connect the Studio plugin to that address, verify project/place and review the
initial patch. Edit one source task, build again, inspect the Studio result and
read the terminal for errors. File creates/renames/deletes are supported; confirm
the resulting paths and final state rather than assuming a rename or queued
request succeeded. Changes to model JSON and metadata are file edits too.

The tutorial's Source and model files remain the normal file inputs. If a human
should own a class's Studio properties, use the explicit `syncAuthority` recipe
in the cookbook. That setting changes what live sync is permitted to replace.

## Bring saved Studio changes back deliberately

Save/export the intended edited place to a separate input file. Commit or back
up the current project, stop competing editors/servers from writing those same
files, and preview the import:

```powershell
& $Rojo syncback $Project --input "$Out\edited.rbxl" --dry-run --list
```

After reviewing the planned paths, apply that exact saved input when intended:

```powershell
& $Rojo syncback $Project --input "$Out\edited.rbxl" -y --list
git diff -- examples/agent-workflow
& $Rojo build $Project -o "$Out\roundtrip.rbxl"
```

Only the second command writes project sources. `--dry-run` calculates the
existing syncback plan; it is not a transaction or a general live property
writer. Existing inline project layout restrictions still apply. Do not treat
all serializer normalization as desired content: review actual property, Source,
reference and metadata changes before keeping the diff.

Optional `twoWaySync` in the plugin supports the implemented Source-only receipt
path for existing reversible bindings. With a matching server, pending is not
persistence; applied/unchanged confirms authoritative readback. Conflicts or
uncertain results require review and retain newer local Source. Explicit matching
class policies revoke those legacy/Source-only writes. Export other Studio-owned
changes through explicit saved-file syncback.

## Verify the example

```powershell
& "$Repo\scripts\verify-agent-workflow.ps1" -Rojo $Rojo
cargo test --locked --test documented_workflow
```

The first command builds an owned temporary place/sourcemap and starts a separate
Studio Edit process for at most45 seconds. It requires the real shared module,
checks its argument contract and verifies metadata, Script RunContexts, typed
containers, model geometry and PrimaryPart. It does not start Run/Play, install
plugins, publish anything or close existing Studio sessions. Artifacts stay under
`target/verification/agent-workflow-*`.

The Rust integration checks the actual example through quoted CLI build,
sourcemap, formatting and dry-run syncback paths. A green Edit smoke/build is
different from gameplay acceptance. Use your game's own test harness and obtain
the appropriate permission before running gameplay or publishing.

## StarterPlayer containers

When building a place file, declare both script containers explicitly. A
StarterPlayer with StarterCharacterScripts but no StarterPlayerScripts can leave
Roblox CoreScripts waiting for the missing sibling. Rojo warns on that condition
in the final built tree, including containers supplied through model files;
it does not insert either container or change project defaults.

```json
{
  "name": "StarterExample",
  "tree": {
    "$className": "DataModel",
    "StarterPlayer": {
      "$className": "StarterPlayer",
      "StarterCharacterScripts": {
        "$className": "StarterCharacterScripts"
      },
      "StarterPlayerScripts": {
        "$className": "StarterPlayerScripts"
      }
    }
  }
}
```

These declarations belong in a `.project.json`/`.project.jsonc` **tree**.
`init.meta.json` describes one filesystem instance; project children and `$path`
inside metadata do not create this hierarchy. Put script `$path` mappings under
the appropriate container in the project, or use a supported model source with
the actual typed containers. A Folder with the same name is not the same class.

Build both representations from the project directory:

```powershell
& $Rojo build '.\default.project.json' -o '.\game.rbxlx'
& $Rojo build '.\default.project.json' -o '.\game.rbxl'
```

The XML and binary warning controls are covered by `tests/starter_player_warning.rs`.
