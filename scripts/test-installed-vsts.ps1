$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$smokeExe = Join-Path $env:LOCALAPPDATA "oscMIDI\cargo-target\debug\vst_smoke.exe"

if (-not (Test-Path $smokeExe)) {
  throw "Missing smoke runner: $smokeExe. Build it first with: cargo build --bin vst_smoke"
}

$plugins = @()
$plugins += Get-ChildItem "C:\Program Files\Common Files\VST3" -Directory -Filter *.vst3 -Recurse -ErrorAction SilentlyContinue |
  ForEach-Object {
    [pscustomobject]@{
      Mode = "--vst3"
      Path = $_.FullName
    }
  }

$vst2Roots = @(
  "C:\Program Files\VSTPlugins",
  "C:\Program Files\Steinberg\VstPlugins",
  "C:\Program Files (x86)\VSTPlugins",
  "C:\Program Files (x86)\Steinberg\VstPlugins"
) | Where-Object { Test-Path $_ }

if ($vst2Roots.Count -gt 0) {
  $plugins += Get-ChildItem $vst2Roots -Recurse -Include *.dll -File -ErrorAction SilentlyContinue |
    ForEach-Object {
      [pscustomobject]@{
        Mode = "--vst2"
        Path = $_.FullName
      }
    }
}

$results = foreach ($plugin in $plugins) {
  $stdoutPath = [System.IO.Path]::GetTempFileName()
  $stderrPath = [System.IO.Path]::GetTempFileName()
  $args = '{0} "{1}"' -f $plugin.Mode, $plugin.Path
  $proc = Start-Process -FilePath $smokeExe -ArgumentList $args -Wait -PassThru -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
  $exitCode = $proc.ExitCode
  $stdoutText = [string](Get-Content -LiteralPath $stdoutPath -Raw)
  $stderrText = [string](Get-Content -LiteralPath $stderrPath -Raw)
  $statusText = if ($exitCode -eq 0) { "ok" } else { "fail" }
  [pscustomobject]@{
    mode = $plugin.Mode
    path = $plugin.Path
    status = $statusText
    exitCode = $exitCode
    stdout = $stdoutText
    stderr = $stderrText
  }

  Remove-Item $stdoutPath, $stderrPath -ErrorAction SilentlyContinue
}

$reportDir = Join-Path $repoRoot "reports"
New-Item -ItemType Directory -Force -Path $reportDir | Out-Null
$timestamp = Get-Date -Format "yyyy-MM-dd_HH-mm-ss"
$reportPath = Join-Path $reportDir "installed-vst-smoke-$timestamp.json"
$results | ConvertTo-Json -Depth 4 | Set-Content -Path $reportPath -Encoding UTF8

$ok = ($results | Where-Object status -eq "ok").Count
$fail = ($results | Where-Object status -eq "fail").Count
$timeout = ($results | Where-Object status -eq "timeout").Count

Write-Host "Smoke report: $reportPath"
Write-Host "OK: $ok  FAIL: $fail  TIMEOUT: $timeout"

$results |
  Sort-Object status, mode, path |
  Select-Object status, mode, path, exitCode |
  Format-Table -AutoSize
