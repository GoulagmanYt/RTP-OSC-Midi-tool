@echo off
setlocal EnableExtensions DisableDelayedExpansion
rem Runs the same colorful pipeline without keeping a persistent build log.
set "OSCMIDI_BUILD_NO_LOG=1"
call "%~dp0build_windows.bat" %*
set "BUILD_EXIT_CODE=%ERRORLEVEL%"
endlocal & exit /b %BUILD_EXIT_CODE%
