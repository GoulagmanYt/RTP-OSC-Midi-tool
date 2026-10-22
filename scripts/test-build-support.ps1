$ErrorActionPreference = 'Stop'
$support = Join-Path $PSScriptRoot 'build-support.ps1'
if (-not (Test-Path -LiteralPath $support)) { throw 'Build support is missing: dependency reuse is not implemented.' }
. $support

function Assert-True($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) "oscmidi-build-tests-$([guid]::NewGuid().ToString('N'))"
$repositoryRoot = Join-Path $fixtureRoot 'workspace with spaces'
$npm = (Get-Command npm.cmd -ErrorAction Stop).Source
$originalLocation = Get-Location
New-Item -ItemType Directory -Path $repositoryRoot -Force | Out-Null
try {
    Set-Location $repositoryRoot
    [IO.File]::WriteAllText((Join-Path $repositoryRoot 'package.json'), '{"name":"build-fixture","version":"1.0.0","private":true}')
    [IO.File]::WriteAllText((Join-Path $repositoryRoot 'package-lock.json'), '{"name":"build-fixture","version":"1.0.0","lockfileVersion":3,"packages":{"":{"name":"build-fixture","version":"1.0.0"}}}')
    Ensure-FrontendDependencies -Npm $npm -NodeVersion 'test-node' -NpmVersion 'test-npm'
    $stampPath = Join-Path $repositoryRoot 'node_modules/.oscmidi-build-dependencies.json'
    Assert-True (Test-Path -LiteralPath $stampPath) 'A successful install must create a dependency stamp.'
    $stampTime = (Get-Item -LiteralPath $stampPath).LastWriteTimeUtc
    Ensure-FrontendDependencies -Npm $npm -NodeVersion 'test-node' -NpmVersion 'test-npm'
    Assert-True ((Get-Item -LiteralPath $stampPath).LastWriteTimeUtc -eq $stampTime) 'An unchanged install must be reused.'
    Set-Content -LiteralPath (Join-Path $repositoryRoot 'node_modules/obsolete.txt') -Value 'old install'
    Ensure-FrontendDependencies -Npm $npm -NodeVersion 'test-node' -NpmVersion 'test-npm' -Force
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot 'node_modules/obsolete.txt'))) 'Forced reinstall must replace the old install.'
    Assert-True (-not (Test-FrontendDependencies -Npm $npm -NodeVersion 'changed-node' -NpmVersion 'test-npm')) 'A Node version change must invalidate dependencies.'
    Assert-True (-not (Test-FrontendDependencies -Npm $npm -NodeVersion 'test-node' -NpmVersion 'changed-npm')) 'An npm version change must invalidate dependencies.'
    Add-Content -LiteralPath (Join-Path $repositoryRoot 'package-lock.json') -Value ' '
    Assert-True (-not (Test-FrontendDependencies -Npm $npm -NodeVersion 'test-node' -NpmVersion 'test-npm')) 'A lockfile change must invalidate dependencies.'
    Ensure-FrontendDependencies -Npm $npm -NodeVersion 'test-node' -NpmVersion 'test-npm'
    [IO.File]::WriteAllText((Join-Path $repositoryRoot 'package.json'), '{"name":"build-fixture","version":"1.0.0","private":true,"dependencies":{"missing-fixture":"file:./does-not-exist"}}')
    $failed = $false
    try { Ensure-FrontendDependencies -Npm $npm -NodeVersion 'test-node' -NpmVersion 'test-npm' *> (Join-Path $fixtureRoot 'expected-install-failure.log') } catch { $failed = $true }
    Assert-True $failed 'An invalid lockfile must stop dependency preparation.'
    Assert-True (-not (Test-Path -LiteralPath $stampPath)) 'A failed install must not leave a valid stamp.'

    # Build tooling is needed even when the caller requests production packages.
    New-Item -ItemType Directory -Path (Join-Path $repositoryRoot 'local-tool') | Out-Null
    Set-Content -LiteralPath (Join-Path $repositoryRoot 'local-tool/package.json') -Value '{"name":"build-dev-tool","version":"1.0.0"}'
    Invoke-CheckedCommand $npm @('pack', './local-tool', '--quiet')
    Set-Content -LiteralPath (Join-Path $repositoryRoot 'package.json') -Value '{"name":"build-fixture","version":"1.0.0","private":true,"devDependencies":{"build-dev-tool":"file:./build-dev-tool-1.0.0.tgz"}}'
    Invoke-CheckedCommand $npm @('install', '--package-lock-only', '--no-audit', '--no-fund')
    $previousNodeEnv = $env:NODE_ENV
    $previousOmit = $env:npm_config_omit
    try {
        $env:NODE_ENV = 'production'
        $env:npm_config_omit = 'dev'
        Ensure-FrontendDependencies -Npm $npm -NodeVersion 'test-node' -NpmVersion 'test-npm'
        Assert-True (Test-Path -LiteralPath (Join-Path $repositoryRoot 'node_modules/build-dev-tool/package.json')) 'Production environments must still install frontend build tooling.'
        $validStamp = Get-Content -LiteralPath $stampPath -Raw
        Invoke-CheckedCommand $npm @('ci', '--omit=dev', '--no-audit', '--no-fund')
        # Reproduce an old stamp written after an omitted development install.
        Set-Content -LiteralPath $stampPath -Value $validStamp
        Assert-True (-not (Test-FrontendDependencies -Npm $npm -NodeVersion 'test-node' -NpmVersion 'test-npm')) 'A stamped install missing build tooling must not be reused in production environments.'
        Ensure-FrontendDependencies -Npm $npm -NodeVersion 'test-node' -NpmVersion 'test-npm'
        Assert-True (Test-Path -LiteralPath (Join-Path $repositoryRoot 'node_modules/build-dev-tool/package.json')) 'An omitted development install must be repaired.'
        Assert-True (Test-FrontendDependencies -Npm $npm -NodeVersion 'test-node' -NpmVersion 'test-npm') 'A complete install must be reusable in production environments.'
    }
    finally {
        $env:NODE_ENV = $previousNodeEnv
        $env:npm_config_omit = $previousOmit
    }

    # Exercise the actual deletion safeguard in an isolated Git repository.
    function Write-BuildLog { param($Message, $Color) }
    Invoke-CheckedCommand git @('init', '--quiet', $repositoryRoot)
    New-Item -ItemType Directory -Path (Join-Path $repositoryRoot '.cargo-target') | Out-Null
    Set-Content -LiteralPath (Join-Path $repositoryRoot '.cargo-target/generated.txt') -Value 'generated'
    Remove-GeneratedDirectory '.cargo-target'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot '.cargo-target'))) 'Explicit cleanup must remove generated caches.'
    New-Item -ItemType Directory -Path (Join-Path $repositoryRoot '.cargo-target') | Out-Null
    Set-Content -LiteralPath (Join-Path $repositoryRoot '.cargo-target/tracked.txt') -Value 'tracked'
    Invoke-CheckedCommand git @('-C', $repositoryRoot, 'add', '--', '.cargo-target/tracked.txt')
    $failed = $false
    try { Remove-GeneratedDirectory '.cargo-target' } catch { $failed = $true }
    Assert-True $failed 'Cleanup must refuse a directory containing tracked files.'
    Assert-True (Test-Path -LiteralPath (Join-Path $repositoryRoot '.cargo-target/tracked.txt')) 'Tracked files must survive refused cleanup.'
    $outside = Join-Path $fixtureRoot 'outside'
    New-Item -ItemType Directory -Path $outside | Out-Null
    $failed = $false
    try { Remove-GeneratedDirectory '..\outside' } catch { $failed = $true }
    Assert-True $failed 'Cleanup must refuse paths outside the workspace.'
    Assert-True (Test-Path -LiteralPath $outside) 'Outside directories must survive refused cleanup.'
    New-Item -ItemType Junction -Path (Join-Path $repositoryRoot 'linked-cache') -Target $outside | Out-Null
    $failed = $false
    try { Remove-GeneratedDirectory 'linked-cache' } catch { $failed = $true }
    Assert-True $failed 'Cleanup must refuse directory junctions.'
    # Remove only the link; never recursively delete through a fixture junction.
    [IO.Directory]::Delete((Join-Path $repositoryRoot 'linked-cache'))

    # Untrack the fixture file so .cargo-target can be tested for full cleanup.
    Invoke-CheckedCommand git @('-C', $repositoryRoot, 'rm', '-f', '--', '.cargo-target/tracked.txt')
    if (Test-Path -LiteralPath (Join-Path $repositoryRoot '.cargo-target')) {
        Remove-Item -LiteralPath (Join-Path $repositoryRoot '.cargo-target') -Recurse -Force
    }

    # Verify Clear-BuildCaches cleans all intermediate directories and handles node_modules appropriately.
    New-Item -ItemType Directory -Force -Path (Join-Path $repositoryRoot '.cargo-target') | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $repositoryRoot 'src-tauri/target') | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $repositoryRoot 'tools/diagnostics/target') | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $repositoryRoot 'tools/latency_bench/target') | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $repositoryRoot 'src-tauri/binaries') | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $repositoryRoot 'dist') | Out-Null
    Clear-BuildCaches
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot '.cargo-target'))) 'Clear-BuildCaches must remove .cargo-target.'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot 'src-tauri/target'))) 'Clear-BuildCaches must remove src-tauri/target.'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot 'tools/diagnostics/target'))) 'Clear-BuildCaches must remove tools/diagnostics/target.'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot 'tools/latency_bench/target'))) 'Clear-BuildCaches must remove tools/latency_bench/target.'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot 'src-tauri/binaries'))) 'Clear-BuildCaches must remove src-tauri/binaries.'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot 'dist'))) 'Clear-BuildCaches must remove dist.'
    Assert-True (Test-Path -LiteralPath (Join-Path $repositoryRoot 'node_modules')) 'Clear-BuildCaches without -IncludeNodeModules must preserve node_modules.'
    Clear-BuildCaches -IncludeNodeModules
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $repositoryRoot 'node_modules'))) 'Clear-BuildCaches -IncludeNodeModules must remove node_modules.'

    # Run the complete cleanup launcher only in a disposable repository.
    $cleanupRoot = Join-Path $fixtureRoot 'cleanup launcher with spaces'
    New-Item -ItemType Directory -Force -Path (Join-Path $cleanupRoot 'scripts') | Out-Null
    $actualRoot = Split-Path -Parent $PSScriptRoot
    foreach ($file in @('clean_all.bat', 'build_windows.bat', 'scripts/build-windows.ps1', 'scripts/build-support.ps1')) {
        Copy-Item -LiteralPath (Join-Path $actualRoot $file) -Destination (Join-Path $cleanupRoot $file)
    }
    Invoke-CheckedCommand git @('init', '--quiet', $cleanupRoot)
    $generatedDirectories = @('.cargo-target', 'src-tauri/target', 'node_modules', 'dist', 'artifacts/logs', 'reports', 'build_logs', 'src-tauri/gen/schemas', 'scripts/__pycache__', 'tools/latency_bench/__pycache__', 'tools/diagnostics/__pycache__', 'tools/diagnostics/tests/__pycache__', 'vendor/rack/target', 'vendor/rtpmidi/target', 'vendor/vst/target', 'vendor/midi-types/target')
    $keptDirectories = @('src', '.vscode', '.playwright-cli', '.venv', 'ASIOSDK', 'vendor/rack/rack-sys/external/vst3sdk')
    foreach ($directory in $generatedDirectories + $keptDirectories) {
        $path = Join-Path $cleanupRoot $directory
        New-Item -ItemType Directory -Force -Path $path | Out-Null
        Set-Content -LiteralPath (Join-Path $path 'fixture.txt') -Value $directory
    }
    Invoke-CheckedCommand git @('-C', $cleanupRoot, 'add', '--', 'src/fixture.txt')
    Invoke-CheckedCommand $env:ComSpec @('/d', '/c', ('"' + (Join-Path $cleanupRoot 'clean_all.bat') + '"'))
    foreach ($directory in $generatedDirectories) {
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $cleanupRoot $directory))) "Full cleanup must remove $directory."
    }
    foreach ($directory in $keptDirectories) {
        Assert-True (Test-Path -LiteralPath (Join-Path $cleanupRoot "$directory/fixture.txt")) "Full cleanup must preserve $directory."
    }
    Assert-True (Test-Path -LiteralPath (Join-Path $cleanupRoot '.git')) 'Full cleanup must preserve Git.'
    # The launcher must fail and preserve tracked files in a generated directory.
    New-Item -ItemType Directory -Force -Path (Join-Path $cleanupRoot '.cargo-target') | Out-Null
    Set-Content -LiteralPath (Join-Path $cleanupRoot '.cargo-target/tracked.txt') -Value 'keep'
    Invoke-CheckedCommand git @('-C', $cleanupRoot, 'add', '--', '.cargo-target/tracked.txt')
    $failed = $false
    try { Invoke-CheckedCommand $env:ComSpec @('/d', '/c', ('"' + (Join-Path $cleanupRoot 'clean_all.bat') + '"')) *> $null } catch { $failed = $true }
    Assert-True $failed 'The cleanup launcher must propagate a refused cleanup exit code.'
    Assert-True (Test-Path -LiteralPath (Join-Path $cleanupRoot '.cargo-target/tracked.txt')) 'Refused full cleanup must preserve tracked files.'

    $failed = $false
    try { Invoke-CheckedCommand $env:ComSpec @('/d', '/c', 'exit 7') } catch { $failed = $true }
    Assert-True $failed 'A failing native command must throw.'
    $tokens = $null; $parseErrors = $null
    $buildAst = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'build-windows.ps1'), [ref]$tokens, [ref]$parseErrors)
    $runner = $buildAst.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Invoke-BuildCommand' }, $true)
    . ([scriptblock]::Create($runner.Extent.Text))
    $buildLogWriter = $null
    Invoke-BuildCommand $env:ComSpec @('/d', '/c', 'exit 0')
    $LASTEXITCODE = 0
    $failed = $false
    try { Invoke-BuildCommand (Join-Path $fixtureRoot 'missing-command.exe') *> $null } catch { $failed = $true }
    Assert-True $failed 'A missing build executable must fail even after an earlier command succeeded.'

    # Rewriting identical Vite output must not force Cargo to relink the app.
    $dist = Join-Path $fixtureRoot 'dist'
    New-Item -ItemType Directory -Path $dist | Out-Null
    $asset = Join-Path $dist 'app.js'
    $index = Join-Path $dist 'index.html'
    [IO.File]::WriteAllText($asset, 'original asset')
    [IO.File]::WriteAllText($index, 'original index')
    $originalTime = [datetime]::SpecifyKind([datetime]'2001-02-03T04:05:06', [DateTimeKind]::Utc)
    foreach ($path in @($asset, $index, $dist)) { (Get-Item -LiteralPath $path).LastWriteTimeUtc = $originalTime }
    $snapshot = Get-FrontendOutputSnapshot $dist
    [IO.File]::WriteAllText($asset, 'original asset')
    [IO.File]::WriteAllText($index, 'original index')
    (Get-Item -LiteralPath $dist).LastWriteTimeUtc = [datetime]::UtcNow
    Restore-FrontendOutputTimestamps $dist $snapshot
    Assert-True ((Get-Item -LiteralPath $asset).LastWriteTimeUtc -eq $originalTime) 'Identical asset bytes must retain the original timestamp.'
    Assert-True ((Get-Item -LiteralPath $dist).LastWriteTimeUtc -eq $originalTime) 'An identical output tree must retain its directory timestamp.'
    [IO.File]::WriteAllText($asset, 'changed asset')
    [IO.File]::WriteAllText($index, 'original index')
    (Get-Item -LiteralPath $dist).LastWriteTimeUtc = [datetime]::UtcNow
    Restore-FrontendOutputTimestamps $dist $snapshot
    Assert-True ((Get-Item -LiteralPath $asset).LastWriteTimeUtc -ne $originalTime) 'Changed asset bytes must remain newer so Cargo rebuilds.'
    Assert-True ((Get-Item -LiteralPath $index).LastWriteTimeUtc -ne $originalTime) 'A changed tree must leave rewritten inputs newer so Cargo regenerates its asset map.'
    Assert-True ((Get-Item -LiteralPath $dist).LastWriteTimeUtc -ne $originalTime) 'A changed output tree must not reuse the old directory timestamp.'
    $snapshot = Get-FrontendOutputSnapshot $dist
    $rewrittenIndexTime = (Get-Item -LiteralPath $index).LastWriteTimeUtc.AddSeconds(1)
    (Get-Item -LiteralPath $index).LastWriteTimeUtc = $rewrittenIndexTime
    [IO.File]::WriteAllText((Join-Path $dist 'new.js'), 'new asset')
    $newTreeTime = [datetime]::UtcNow.AddSeconds(1)
    (Get-Item -LiteralPath $dist).LastWriteTimeUtc = $newTreeTime
    Restore-FrontendOutputTimestamps $dist $snapshot
    Assert-True ((Get-Item -LiteralPath $dist).LastWriteTimeUtc -eq $newTreeTime) 'A new asset must invalidate the directory timestamp.'
    Assert-True ((Get-Item -LiteralPath $index).LastWriteTimeUtc -eq $rewrittenIndexTime) 'A new asset must keep an existing tracked input newer even when Cargo only tracks old files.'
    $snapshot = Get-FrontendOutputSnapshot $dist
    Remove-Item -LiteralPath (Join-Path $dist 'new.js')
    $deletedTreeTime = $newTreeTime.AddSeconds(1)
    (Get-Item -LiteralPath $dist).LastWriteTimeUtc = $deletedTreeTime
    Restore-FrontendOutputTimestamps $dist $snapshot
    Assert-True ((Get-Item -LiteralPath $dist).LastWriteTimeUtc -eq $deletedTreeTime) 'A removed asset must invalidate the directory timestamp.'
    $snapshot = Get-FrontendOutputSnapshot $dist
    Move-Item -LiteralPath $index -Destination (Join-Path $dist 'renaming.tmp')
    Move-Item -LiteralPath (Join-Path $dist 'renaming.tmp') -Destination (Join-Path $dist 'INDEX.HTML')
    (Get-Item -LiteralPath $dist).LastWriteTimeUtc = $deletedTreeTime.AddSeconds(1)
    Restore-FrontendOutputTimestamps $dist $snapshot
    Assert-True ((Get-Item -LiteralPath $dist).LastWriteTimeUtc -ne $deletedTreeTime) 'Case-only asset renames must invalidate the embedded asset names.'
    New-Item -ItemType Junction -Path (Join-Path $dist 'linked-assets') -Target $outside | Out-Null
    Assert-True ($null -eq (Get-FrontendOutputSnapshot $dist)) 'Output timestamp reuse must refuse linked entries.'
    [IO.Directory]::Delete((Join-Path $dist 'linked-assets'))
    Write-Host 'PASS: dependency reuse/invalidation, failed installs, cleanup boundaries/tracked files, native exit codes.'
}
finally {
    Set-Location $originalLocation
    $resolvedFixture = [IO.Path]::GetFullPath($fixtureRoot)
    if (-not $resolvedFixture.StartsWith([IO.Path]::GetTempPath(), [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe test cleanup path.' }
    Remove-Item -LiteralPath $resolvedFixture -Recurse -Force
}
