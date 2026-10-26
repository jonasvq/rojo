#Requires -Version 7.0
param([switch]$ProjectTypesOnly)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
Push-Location $repoRoot
try {
    $graphArgs = @('tree', '--locked', '--edges', 'normal,build', '--prefix', 'none', '--format', '{p}')
    if ($ProjectTypesOnly) { $graphArgs += @('-p', 'rojo-project-types') }
    else { $graphArgs += '--no-default-features' }
    $graph = & cargo @graphArgs
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the locked library dependency graph' }
    $forbidden = $graph | Where-Object { $_ -match '^(clap|hyper|hyper-tungstenite|reqwest|tokio|roblox_install|ritz|opener|backtrace) v' } | Sort-Object -Unique
    if ($ProjectTypesOnly) {
        $forbidden += $graph | Where-Object { $_ -match '^(rojo|memofs|notify|rbx_binary|rbx_xml|crossbeam-channel|jod-thread|futures) v' } | Sort-Object -Unique
    }
    if ($forbidden) { throw ('CLI dependencies remain in the library graph: ' + ($forbidden -join ', ')) }
    $packages = $graph | Where-Object { $_ -match '^[a-zA-Z0-9_-]+ v[0-9]' } | ForEach-Object { $_ -replace ' \(\*\)$', '' } | Sort-Object -Unique
    Write-Output ('LIBRARY_DEPENDENCIES_PASS unique normal/build packages=' + $packages.Count)
} finally {
    Pop-Location
}
