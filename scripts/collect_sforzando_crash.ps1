param(
  [int]$TailLines = 500
)

$ErrorActionPreference = 'SilentlyContinue'
$configDir = Join-Path $env:APPDATA 'OSCMIDI\OSCMIDI\config'
$reportsDir = 'D:\PROGRAMMATION\oscMIDI\reports\crash'
$ts = Get-Date -Format 'yyyy-MM-dd_HH-mm-ss'

New-Item -ItemType Directory -Force -Path $reportsDir | Out-Null
$outFile = Join-Path $reportsDir ("sforzando-crash-" + $ts + ".txt")

"=== OSCMIDI Crash Report ($ts) ===" | Set-Content -Path $outFile -Encoding UTF8
"" | Add-Content -Path $outFile

"[Paths]" | Add-Content -Path $outFile
"APPDATA: $env:APPDATA" | Add-Content -Path $outFile
"ConfigDir: $configDir" | Add-Content -Path $outFile
"" | Add-Content -Path $outFile

$appLog = Join-Path $configDir 'app.log'
$nativeLog = Join-Path $configDir 'vst3_native.log'

"[app.log - last $TailLines lines]" | Add-Content -Path $outFile
if (Test-Path $appLog) {
  Get-Content -Path $appLog -Tail $TailLines | Add-Content -Path $outFile
} else {
  "app.log missing" | Add-Content -Path $outFile
}
"" | Add-Content -Path $outFile

"[vst3_native.log - full]" | Add-Content -Path $outFile
if (Test-Path $nativeLog) {
  Get-Content -Path $nativeLog | Add-Content -Path $outFile
} else {
  "vst3_native.log missing" | Add-Content -Path $outFile
}
"" | Add-Content -Path $outFile

"[Windows Event Log - Application Error 1000 for OSCMidi.exe]" | Add-Content -Path $outFile
Get-WinEvent -FilterHashtable @{LogName='Application'; ID=1000; StartTime=(Get-Date).AddDays(-2)} |
  Where-Object { $_.Message -match 'OSCMidi.exe' } |
  Select-Object -First 8 |
  ForEach-Object {
    "TIME: $($_.TimeCreated)" | Add-Content -Path $outFile
    $_.Message | Add-Content -Path $outFile
    "---" | Add-Content -Path $outFile
  }

Write-Output "Crash report generated: $outFile"
