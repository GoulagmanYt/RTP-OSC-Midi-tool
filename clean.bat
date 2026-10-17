@echo off
setlocal EnableExtensions
title OSCMidi - Clean Workspace
cd /d "%~dp0"
call "%~dp0build_windows.bat" -CleanOnly %*
exit /b %ERRORLEVEL%
