@echo off
setlocal EnableExtensions DisableDelayedExpansion
title OSCMidi - Build Windows
cd /d "%~dp0"
chcp 65001 >nul 2>nul

set "NO_COLOR="
set "FORCE_COLOR=1"
set "CARGO_TERM_COLOR=always"
set "BUILD_SCRIPT=%~dp0scripts\build-windows.ps1"
set "BUILD_EXIT_CODE=1"

if defined OSCMIDI_BUILD_NO_LOG (
  set "OSCMIDI_BUILD_SUBTITLE=Automatic cleanup - no persistent build log"
) else (
  set "OSCMIDI_BUILD_SUBTITLE=Automatic cleanup - detailed build log enabled"
)

set "POWERSHELL_EXE=pwsh.exe"
where pwsh.exe >nul 2>nul || set "POWERSHELL_EXE=powershell.exe"

if not exist "%BUILD_SCRIPT%" (
  echo ERROR: build script not found:
  echo   %BUILD_SCRIPT%
  set "BUILD_EXIT_CODE=2"
  goto :build_finished
)

"%POWERSHELL_EXE%" -NoLogo -NoProfile -Command "$line = '=' * 60; Write-Host ''; Write-Host $line -ForegroundColor DarkCyan; Write-Host '  OSCMidi - Windows Build' -ForegroundColor Cyan; Write-Host ('  ' + $env:OSCMIDI_BUILD_SUBTITLE) -ForegroundColor DarkGray; Write-Host $line -ForegroundColor DarkCyan; Write-Host ''"

"%POWERSHELL_EXE%" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%BUILD_SCRIPT%" %*
set "BUILD_EXIT_CODE=%ERRORLEVEL%"

:build_finished
echo.
if "%BUILD_EXIT_CODE%"=="0" (
  "%POWERSHELL_EXE%" -NoLogo -NoProfile -Command "Write-Host '[OK] Build completed successfully.' -ForegroundColor Green; Write-Host '     Artifacts: %~dp0artifacts' -ForegroundColor Cyan"
) else (
  "%POWERSHELL_EXE%" -NoLogo -NoProfile -Command "Write-Host '[FAIL] Build exited with code %BUILD_EXIT_CODE%.' -ForegroundColor Red; $log = Get-ChildItem -LiteralPath '%~dp0artifacts\logs' -Filter 'build-*.log' -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1; if ($log) { Write-Host ('       Log: ' + $log.FullName) -ForegroundColor Yellow } elseif ($env:OSCMIDI_BUILD_NO_LOG -eq '1') { Write-Host '       Persistent logging was disabled by build_windows_no_log.bat.' -ForegroundColor Yellow } else { Write-Host '       No log was created; check the PowerShell launch error above.' -ForegroundColor Yellow }"
)

endlocal & exit /b %BUILD_EXIT_CODE%
