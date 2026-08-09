@echo off
rem Alias conserve pour compatibilite : tous les builds utilisent maintenant
rem la meme pipeline avec progression, journal et nettoyage automatique.
call "%~dp0build_windows.bat" %*
exit /b %ERRORLEVEL%
