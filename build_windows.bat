@echo off
setlocal EnableExtensions
title OSCMidi - Build Windows
cd /d "%~dp0"

set "POWERSHELL_EXE=pwsh.exe"
where pwsh.exe >nul 2>nul || set "POWERSHELL_EXE=powershell.exe"

echo ============================================================
echo   OSCMidi - Build Windows avec nettoyage automatique
echo ============================================================
echo.

"%POWERSHELL_EXE%" -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\build-windows.ps1"
set "BUILD_EXIT_CODE=%ERRORLEVEL%"

echo.
if "%BUILD_EXIT_CODE%"=="0" (
  echo Build termine avec succes.
) else (
  echo Build termine avec le code d'erreur %BUILD_EXIT_CODE%.
)
echo Livrables et journaux : %~dp0artifacts

exit /b %BUILD_EXIT_CODE%
