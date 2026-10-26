# Windows CLI and Studio setup

Use PowerShell7 and the local executable. Installing the Studio plugin does not
install a terminal command or add anything to PATH. The VSCode extension has its
own release/version history; its version is not the CLI version, the Studio
plugin version or the wire protocol number. Claude Code can use the CLI directly.

## Identify the executable before changing anything

From the repository root:

```powershell
$Rojo = (Resolve-Path -LiteralPath '.\target\debug\rojo.exe').Path
& $Rojo --version
Get-Command rojo -All -ErrorAction SilentlyContinue | Select-Object CommandType, Source
Get-FileHash -LiteralPath $Rojo -Algorithm SHA256
git rev-parse --short HEAD
```

`rojo --version` reports8.0.0 for this fork. That number alone does not identify
the local changes: record the Git revision and executable hash when comparing
machines. `Get-Command` shows which executable/shim an unqualified `rojo` would
run. An absolute invocation avoids an unrelated global installation or old shim.

If the local executable is absent or stale, build this repository explicitly:

```powershell
cargo build --locked
& '.\target\debug\rojo.exe' --version
& '.\target\debug\rojo.exe' serve --help
```

This checkout's `rokit.toml` pins this fork as `jonasvq/rojo@8.0.0`. Its other tool pins
are Selene0.29.0, StyLua2.1.0, run-in-roblox0.3.0 and Lune0.10.4. Older checkouts
pinned upstream Rojo7.5.1; do not use that binary for this fork. If a tool-manager
shim fails, inspect its project pin and use an explicit executable path to verify
the intended version before changing your installation.

## Start the server; install the matching plugin separately

```powershell
& $Rojo serve 'C:\projects\my game\default.project.json' --address 127.0.0.1 --port 34872
```

The command is `serve`, not `serv`. Keep the terminal running. In Studio, connect
the Rojo plugin to `127.0.0.1:34872`, verify the project/place and inspect the
initial diff. The server prints an address only after successful binding.

To install the plugin embedded in this exact executable, review the local command
and run it when replacing your Studio plugin is intended:

```powershell
& $Rojo plugin install --help
& $Rojo plugin install
```

The install command writes the Studio plugin; the diagnostic checks in this guide
do not run it. Protocol5 plus negotiated capabilities decide compatibility.
Features such as class authority, Source receipts and automatic debugger plans
need their matching local plugin/server implementation even when version numbers
look the same. Update both from the same local build, then reconnect.

## If PowerShell says the command was not found

First try the absolute form:

```powershell
& '.\target\debug\rojo.exe' --version
```

If it works, the executable is present; the problem is command discovery/PATH,
not live sync. Continue with the absolute path. If you choose to configure PATH
yourself later, add the **directory** containing the intended executable and open
a fresh terminal; existing terminals may retain their older environment. Merely
installing a Studio or VSCode plugin does not make this CLI discoverable.

If the absolute path fails, verify it with `Test-Path -LiteralPath`, build the
repository and inspect that build's error first. If `rojo` runs the wrong version,
check every result of `Get-Command rojo -All` rather than reinstalling blindly.
If the server starts but connection fails, check its port, place restrictions
and handshake diagnostic; changing PATH will not repair those failures.

Focused verification: `scripts/verify-local-cli.ps1` runs a clean-PATH child shell
and a copied local executable under paths with spaces. It changes no installed
plugin, user PATH, user files or existing Studio session.
