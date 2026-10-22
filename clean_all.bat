@echo off
setlocal EnableExtensions DisableDelayedExpansion
rem Removes generated repository outputs and dependencies without rebuilding.
call "%~dp0build_windows.bat" -FullClean
set "CLEAN_EXIT_CODE=%ERRORLEVEL%"
endlocal & exit /b %CLEAN_EXIT_CODE%
