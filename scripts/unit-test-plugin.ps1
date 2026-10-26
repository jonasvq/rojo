param(
    [string]$Rojo,
    [string]$Runner
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
if (-not $Rojo) { $Rojo = Join-Path $repoRoot 'target/debug/rojo.exe' }
if (-not $Runner) { $Runner = Join-Path $repoRoot 'target/tools/run-in-roblox-0.3.0/unpacked/run-in-roblox.exe' }
$Rojo = (Resolve-Path -LiteralPath $Rojo).Path
$Runner = (Resolve-Path -LiteralPath $Runner).Path
$run = Join-Path $repoRoot ('target/verification/plugin-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $run | Out-Null
$place = Join-Path $run 'TestPlace.rbxl'
& $Rojo build (Join-Path $repoRoot 'plugin/test-place.project.json') --output $place
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$start = [Diagnostics.ProcessStartInfo]::new($Runner)
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
foreach ($argument in @('--script', (Join-Path $repoRoot 'plugin/run-tests.server.lua'), '--place', $place)) {
    $start.ArgumentList.Add($argument)
}
$process = [Diagnostics.Process]::new()
$process.StartInfo = $start
$null = $process.Start()
$stdout = $process.StandardOutput.ReadToEndAsync()
$stderr = $process.StandardError.ReadToEndAsync()
$finished = $false
try { $finished = $process.WaitForExit(45000) }
finally {
    if (-not $process.HasExited) { $process.Kill($true); $process.WaitForExit() }
    $output = $stdout.GetAwaiter().GetResult()
    $errors = $stderr.GetAwaiter().GetResult()
    [IO.File]::WriteAllText((Join-Path $run 'stdout.log'), $output)
    [IO.File]::WriteAllText((Join-Path $run 'stderr.log'), $errors)
    Write-Output "Artifacts: $run"
    $output -split "`n" | Select-Object -Last 15 | Write-Output
    if ($errors) { Write-Output $errors }
}
if (-not $finished) { throw 'Studio tests exceeded 45 seconds' }
if ($process.ExitCode -ne 0) { exit $process.ExitCode }
if ($output -notmatch '([1-9][0-9]*) passed, 0 failed') { throw 'Studio did not report a successful nonempty TestEZ run' }
$process.Dispose()
