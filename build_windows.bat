@echo off
setlocal ENABLEEXTENSIONS

set "CARGO_TARGET_DIR=%~dp0.cargo-target"

rem Build OSCMidi Bridge on Windows (Vite + Tauri)
set "SCRIPT_DIR=%~dp0"
cd /d "%SCRIPT_DIR%"

set "LOG_DIR=%SCRIPT_DIR%build_logs"
if not exist "%LOG_DIR%" mkdir "%LOG_DIR%" >nul 2>nul
set "TS=%DATE:~6,4%-%DATE:~3,2%-%DATE:~0,2%_%TIME:~0,2%-%TIME:~3,2%-%TIME:~6,2%"
set "TS=%TS: =0%"
set "LOG_FILE=%LOG_DIR%\build-%TS%.log"
set "BUILD_STATUS=RUN"
set "EXIT_CODE=0"
echo Journal : %LOG_FILE%
echo --- Build OSCMidi (%DATE% %TIME%) --- >"%LOG_FILE%"

:RUN_BUILD
where npm >nul 2>nul || goto NONODE
where cargo >nul 2>nul || where rustup >nul 2>nul || goto NORUST

call :LOG "[1/4] Install deps (npm ci)..."
if exist node_modules (
  call :RUNCMD npm install --no-fund --no-audit --silent || goto ERROR
) else (
  call :RUNCMD npm ci --no-fund --no-audit || goto ERROR
)

call :LOG "[2/4] Build UI (Vite)..."
call :RUNCMD npm run build || goto ERROR

call :LOG "[3/4] Bundle Tauri (.exe + .msi)..."
set TAURI_BUNDLE_TARGETS=app,msi
call :RUNCMD npm run tauri:build || goto ERROR

if exist "%CARGO_TARGET_DIR%\release\osc-midi-bridge.exe" (
  del /f /q "%CARGO_TARGET_DIR%\release\osc-midi-bridge.exe" >>"%LOG_FILE%" 2>&1
)
if exist "%CARGO_TARGET_DIR%\release\vst_smoke.exe" (
  del /f /q "%CARGO_TARGET_DIR%\release\vst_smoke.exe" >>"%LOG_FILE%" 2>&1
)

call :LOG "[4/4] Build termine. Artefacts : %CARGO_TARGET_DIR%\release\bundle\ et %CARGO_TARGET_DIR%\release\OSCMidi.exe"
set "BUILD_STATUS=OK"
set "EXIT_CODE=0"
goto FINALIZE

:NONODE
call :LOG "Node.js ou npm introuvable. Installez Node.js (>=20) puis relancez."
set "BUILD_STATUS=FAIL"
set "EXIT_CODE=1"
goto FINALIZE

:NORUST
call :LOG "Rust/Cargo introuvable. Installez rustup (https://rustup.rs) puis relancez."
set "BUILD_STATUS=FAIL"
set "EXIT_CODE=1"
goto FINALIZE

:ERROR
set "BUILD_STATUS=FAIL"
set "EXIT_CODE=%ERRORLEVEL%"
goto FINALIZE

:FINALIZE
set "FINAL_LOG=%LOG_DIR%\build-%TS%-%BUILD_STATUS%.log"
if /i not "%LOG_FILE%"=="%FINAL_LOG%" (
  ren "%LOG_FILE%" "build-%TS%-%BUILD_STATUS%.log"
)
echo Journal final : %FINAL_LOG%
exit /b %EXIT_CODE%

:LOG
echo %~1
echo %~1>>"%LOG_FILE%"
goto :eof

:RUNCMD
echo %*
echo %*>>"%LOG_FILE%"
call %* >>"%LOG_FILE%" 2>&1
goto :eof

endlocal

rem Maintenir l'affichage live tout en loggant : ne pas sortir des labels au-dessus
