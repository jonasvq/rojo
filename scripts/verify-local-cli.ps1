#Requires -Version 7.0
param([string]$Rojo)
$ErrorActionPreference='Stop'
$repoRoot=Split-Path $PSScriptRoot -Parent
if (-not $Rojo) { $Rojo=Join-Path $repoRoot 'target/debug/rojo.exe' }
$Rojo=(Resolve-Path -LiteralPath $Rojo).Path
$run=Join-Path $repoRoot ('target/verification/local-cli-' + [Guid]::NewGuid().ToString('N'))
$owned=Join-Path $run 'folder with spaces'
New-Item -ItemType Directory -Path $owned | Out-Null
$copy=Join-Path $owned 'local rojo.exe'
Copy-Item -LiteralPath $Rojo -Destination $copy
$childScript=Join-Path $owned 'clean shell.ps1'
[IO.File]::WriteAllText($childScript, @'
param([string]$Rojo)
$ErrorActionPreference='Stop'
if (Get-Command rojo -ErrorAction SilentlyContinue) { throw 'Unexpected rojo command in clean shell' }
& $Rojo --version
exit $LASTEXITCODE
'@)
$start=[Diagnostics.ProcessStartInfo]::new((Get-Process -Id $PID).Path)
$start.UseShellExecute=$false
$start.CreateNoWindow=$true
$start.RedirectStandardOutput=$true
$start.RedirectStandardError=$true
$start.Environment['PATH']=''
foreach ($argument in @('-NoProfile','-NonInteractive','-File',$childScript,'-Rojo',$copy)) { $start.ArgumentList.Add($argument) }
$process=[Diagnostics.Process]::new()
$process.StartInfo=$start
$null=$process.Start()
$stdout=$process.StandardOutput.ReadToEndAsync()
$stderr=$process.StandardError.ReadToEndAsync()
$finished=$false
try { $finished=$process.WaitForExit(15000) }
finally { if (-not $process.HasExited) { $process.Kill($true); $process.WaitForExit() } }
$out=$stdout.GetAwaiter().GetResult()
$err=$stderr.GetAwaiter().GetResult()
[IO.File]::WriteAllText((Join-Path $run 'stdout.log'),$out)
[IO.File]::WriteAllText((Join-Path $run 'stderr.log'),$err)
if (-not $finished -or $process.ExitCode -ne 0 -or $out -notmatch '\b8\.0\.0\b') { throw "Clean-PATH explicit invocation failed: $out $err" }
$process.Dispose()
$project=Join-Path $owned 'example project.project.json'
[IO.File]::WriteAllText($project,'{"name":"QuotedPaths","tree":{"$className":"Folder"}}')
$output=Join-Path $owned 'output model.rbxmx'
& $copy build $project --output $output
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $output)) { throw 'Quoted path build failed' }
Write-Output "Artifacts: $run"
Write-Output 'LOCAL_CLI_VERIFIED clean-PATH miss; explicit local version; spaced executable/project/output paths'
