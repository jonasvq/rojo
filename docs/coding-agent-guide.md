# Coding-agent guide: Claude Code and other local agents

Use the actual files and CLI as evidence. Start with
[local setup](windows-local-install.md), the [workflow](fully-managed-workflow.md)
and [cookbook](rojo-cookbook.md). The checked-in example is
`examples/agent-workflow/default.project.json`.

## Make the guide visible to your agent

Installing the CLI does not load Markdown into an agent's context. This
repository's `CLAUDE.md` imports this guide. In a game project, explicitly
reference the guide or add its path to that project's instructions. Other
agents follow their own instruction discovery rules.

For a task, explicitly ask the agent to read this guide, the workflow and the
cookbook. In Claude Code, attach `@docs/coding-agent-guide.md` from this checkout.
To keep it available in a game project, put an `@` import of the actual guide
path in that project's own `CLAUDE.md`. Keep the guide and its relative links
together; the release's guides archive includes the docs and example.

## A small, verifiable edit loop

1. Read the project's real tree, source mappings, version, sync rules, metadata
   and authority before choosing files. Locate symbols with `rg`; a Studio name
   is not necessarily its source filename or an authorization to adopt it.
2. Make one intended behavior concrete with an appropriate failing case. Edit
   source files, not a generated binary or serializer output unless that model
   is itself the reviewed source asset.
3. Build with the exact local executable and project path. Inspect the sourcemap
   for class/RunContext/path mistakes. JSON and model fields are case-sensitive
   contracts; use the canonical examples rather than guessing metadata keys.
4. Run the relevant module/client/native check. A build doesn't run gameplay.
   `scripts/verify-agent-workflow.ps1` checks the example in a disposable Edit
   process; it doesn't authorize Play, Team Create or publication.
5. Review `git diff`, actual Studio identity/properties and diagnostics. Report
   the test's scope, result and any genuinely unverified runtime behavior.

For the example, from this repository in PowerShell7:

```powershell
$Rojo = (Resolve-Path -LiteralPath '.\target\debug\rojo.exe').Path
$Project = '.\examples\agent-workflow\default.project.json'
& $Rojo build $Project -o '.\target\agent-example.rbxlx'
& $Rojo sourcemap $Project --include-non-scripts --relative-to-output -o '.\target\agent-example-sourcemap.json'
cargo test --locked --test documented_workflow
& '.\scripts\verify-agent-workflow.ps1' -Rojo $Rojo
```

The Cargo command verifies this repository's example/CLI integration. In a game
repository without this Rust workspace, use that project's own test harness;
don't invent a Cargo test or silently install a different Rojo version.

## Protect data while using Studio

- File-owned values follow files. Studio-owned class fields are preserved live
  and exported through explicit saved-file syncback. Read the
  [class-authority reference](class-authority.md) before changing ownership.
- Source receipt queue admission is pending, not persistence. Keep newer local
  Source when a result is conflicting/uncertain and require explicit review.
- Stop on ambiguous identity, missing/rejected parent, unsafe replacement or
  changed authority. Inspect the actual cause; don't erase children, clone by
  name or disable safeguards to force a green-looking sync.
- Use `syncback --dry-run --list` before an intended import. Apply only a reviewed
  saved input, inspect changes and rebuild. Don't run competing source writers.

## Testing changes to Rojo itself

Follow `AGENTS.md` for repository conventions. Use the relevant focused tests,
then the workspace tests, formatting and lint checks. Preserve unrelated local
changes. Build the executable explicitly and verify its version before testing;
do not replace it while another test is using it. Native test helpers use bounded
disposable Studio processes and preserve existing sessions. Consult CLI help
and implemented capability contracts rather than guessing API behavior.
