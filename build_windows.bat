@echo off
setlocal ENABLEEXTENSIONS

set "CARGO_TARGET_DIR=%~dp0.cargo-target"

rem Build OSCMidi Bridge on Windows (Vite + Tauri)
cd /d "%~dp0" >nul 2>&1
for %%I in ("%CD%") do set "SCRIPT_DIR=%%~fI\"

rem Ensure required Windows build tools are available for Rust/Tauri dependencies
set "VSCMD_PATH="
if exist "%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" (
  for /f "usebackq delims=" %%I in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSCMD_PATH=%%I\Common7\Tools\VsDevCmd.bat"
)
if not defined VSCMD_PATH if exist "%ProgramFiles%\Microsoft Visual Studio\Installer\vswhere.exe" (
  for /f "usebackq delims=" %%I in (`"%ProgramFiles%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSCMD_PATH=%%I\Common7\Tools\VsDevCmd.bat"
)
if exist "%VSCMD_PATH%" (
  call "%VSCMD_PATH%" -arch=x64 >nul 2>&1
)

set "CMAKE_BIN="
for %%I in (cmake.exe) do (
  for %%P in ("%%~$PATH:I") do set "CMAKE_BIN=%%~fP"
)
if not defined CMAKE_BIN if exist "%ProgramFiles(x86)%\Microsoft Visual Studio\18\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe" set "CMAKE_BIN=%ProgramFiles(x86)%\Microsoft Visual Studio\18\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
if not defined CMAKE_BIN if exist "%ProgramFiles%\CMake\bin\cmake.exe" set "CMAKE_BIN=%ProgramFiles%\CMake\bin\cmake.exe"
if not defined CMAKE_BIN if exist "%LOCALAPPDATA%\Programs\CMake\bin\cmake.exe" set "CMAKE_BIN=%LOCALAPPDATA%\Programs\CMake\bin\cmake.exe"
if not defined CMAKE_BIN if exist "%USERPROFILE%\.mozbuild\cmake\bin\cmake.exe" set "CMAKE_BIN=%USERPROFILE%\.mozbuild\cmake\bin\cmake.exe"
if defined CMAKE_BIN (
  for %%D in ("%CMAKE_BIN%") do set "CMAKE_DIR=%%~dpD"
  set "PATH=%CMAKE_DIR%;%PATH%"
)

set "CLANG_BIN="
for %%I in (clang.exe) do (
  for %%P in ("%%~$PATH:I") do set "CLANG_BIN=%%~fP"
)
if not defined CLANG_BIN if exist "%ProgramFiles%\LLVM\bin\clang.exe" set "CLANG_BIN=%ProgramFiles%\LLVM\bin\clang.exe"
if not defined CLANG_BIN if exist "%ProgramFiles(x86)%\LLVM\bin\clang.exe" set "CLANG_BIN=%ProgramFiles(x86)%\LLVM\bin\clang.exe"
if not defined CLANG_BIN if exist "%LOCALAPPDATA%\Programs\LLVM\bin\clang.exe" set "CLANG_BIN=%LOCALAPPDATA%\Programs\LLVM\bin\clang.exe"
if not defined CLANG_BIN if exist "%USERPROFILE%\.mozbuild\clang\bin\clang.exe" set "CLANG_BIN=%USERPROFILE%\.mozbuild\clang\bin\clang.exe"
if defined CLANG_BIN (
  for %%D in ("%CLANG_BIN%") do set "CLANG_DIR=%%~dpD"
  set "PATH=%CLANG_DIR%;%PATH%"
  set "CLANG_PATH=%CLANG_BIN%"
)

set "LIBCLANG_PATH="
for %%I in (libclang.dll) do (
  for %%P in ("%%~$PATH:I") do set "LIBCLANG_PATH=%%~dpP"
)
if not defined LIBCLANG_PATH if exist "%ProgramFiles%\LLVM\bin\libclang.dll" set "LIBCLANG_PATH=%ProgramFiles%\LLVM\bin"
if not defined LIBCLANG_PATH if exist "%ProgramFiles(x86)%\LLVM\bin\libclang.dll" set "LIBCLANG_PATH=%ProgramFiles(x86)%\LLVM\bin"
if not defined LIBCLANG_PATH if exist "%LOCALAPPDATA%\Programs\LLVM\bin\libclang.dll" set "LIBCLANG_PATH=%LOCALAPPDATA%\Programs\LLVM\bin"
if not defined LIBCLANG_PATH if exist "%USERPROFILE%\.mozbuild\clang\bin\libclang.dll" set "LIBCLANG_PATH=%USERPROFILE%\.mozbuild\clang\bin"
if defined LIBCLANG_PATH (
  set "LIBCLANG_PATH=%LIBCLANG_PATH%"
)

if not defined CMAKE_BIN (
  call :LOG "CMake introuvable. Installez CMake (par exemple via winget: winget install Kitware.CMake)."
  set "BUILD_STATUS=FAIL"
  set "EXIT_CODE=1"
  goto FINALIZE
)
if not defined CLANG_BIN (
  call :LOG "Clang introuvable. Installez LLVM avec clang/libclang (par exemple via winget: winget install LLVM.LLVM)."
  set "BUILD_STATUS=FAIL"
  set "EXIT_CODE=1"
  goto FINALIZE
)
if not defined LIBCLANG_PATH (
  call :LOG "libclang.dll introuvable. Installez LLVM avec libclang (par exemple via winget: winget install LLVM.LLVM)."
  set "BUILD_STATUS=FAIL"
  set "EXIT_CODE=1"
  goto FINALIZE
)

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
