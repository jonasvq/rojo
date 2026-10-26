#Requires -Version 7.0
param([string]$Rojo)
$ErrorActionPreference='Stop'
$repoRoot=Split-Path $PSScriptRoot -Parent
if (-not $Rojo) { $Rojo=Join-Path $repoRoot 'target/debug/rojo.exe' }
$Rojo=(Resolve-Path -LiteralPath $Rojo).Path
$project=Join-Path $repoRoot 'examples/agent-workflow/default.project.json'
$run=Join-Path $repoRoot ('target/verification/agent-workflow-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $run | Out-Null
$place=Join-Path $run 'example place.rbxl'
& $Rojo build $project -o $place
if ($LASTEXITCODE -ne 0) { throw 'Tutorial place build failed' }
& $Rojo sourcemap $project --include-non-scripts --relative-to-output -o (Join-Path $run 'sourcemap.json')
if ($LASTEXITCODE -ne 0) { throw 'Tutorial sourcemap failed' }
$start=[Diagnostics.ProcessStartInfo]::new((Join-Path $repoRoot 'target/tools/run-in-roblox-0.3.0/unpacked/run-in-roblox.exe'))
$start.UseShellExecute=$false
$start.CreateNoWindow=$true
$start.RedirectStandardOutput=$true
$start.RedirectStandardError=$true
foreach ($argument in @('--script',(Join-Path $PSScriptRoot 'verify-agent-workflow.server.lua'),'--place',$place)) { $start.ArgumentList.Add($argument) }
$process=[Diagnostics.Process]::new()
$process.StartInfo=$start
$null=$process.Start()
$stdout=$process.StandardOutput.ReadToEndAsync()
$stderr=$process.StandardError.ReadToEndAsync()
$finished=$false
try { $finished=$process.WaitForExit(45000) }
finally { if (-not $process.HasExited) { $process.Kill($true); $process.WaitForExit() } }
$out=$stdout.GetAwaiter().GetResult()
$err=$stderr.GetAwaiter().GetResult()
[IO.File]::WriteAllText((Join-Path $run 'stdout.log'),$out)
[IO.File]::WriteAllText((Join-Path $run 'stderr.log'),$err)
Write-Output "Artifacts: $run"
$out -split "`n" | Where-Object { $_ -match '^AGENT_WORKFLOW_VERIFIED' } | Write-Output
if (-not $finished -or $process.ExitCode -ne 0 -or $out -notmatch 'AGENT_WORKFLOW_VERIFIED') { throw "Tutorial native smoke test failed: $err" }
$process.Dispose()
