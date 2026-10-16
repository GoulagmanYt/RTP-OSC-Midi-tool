$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'build-support.ps1')
$previousLocation = Get-Location
$previousChildMode = $env:OSCMIDI_FRONTEND_NODE_CHILD
try {
    Set-Location $repositoryRoot
    $dist = Join-Path $repositoryRoot 'dist'
    $snapshot = Get-FrontendOutputSnapshot $dist
    # Keep arbitrary CLI arguments inside JSON until Node can forward them;
    # PowerShell 5's native marshalling corrupts quoted trailing backslashes.
    $env:OSCMIDI_FRONTEND_NODE_CHILD = '1'
    Invoke-CheckedCommand node @((Join-Path $PSScriptRoot 'build-frontend.mjs'))
    Restore-FrontendOutputTimestamps $dist $snapshot
}
finally {
    $env:OSCMIDI_FRONTEND_NODE_CHILD = $previousChildMode
    Set-Location $previousLocation
}
