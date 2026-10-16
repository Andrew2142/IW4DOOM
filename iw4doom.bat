@echo off
rem SPDX-License-Identifier: GPL-3.0-or-later
rem IW4DOOM for Windows. Double-click this file, or run it from a terminal with options
rem (iw4doom.bat --help). Put your game files in games\ first (see games\README.md).
rem The first run builds everything (15-20 minutes); later runs start the game in seconds.
rem The logic is in iw4doom.ps1; -ExecutionPolicy Bypass applies to this one run only.
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0iw4doom.ps1" %*
set "IW4DOOM_EXIT=%ERRORLEVEL%"
rem Keep the window open on failure so a double-click user can read the message.
if not "%IW4DOOM_EXIT%"=="0" if not defined CI pause
exit /b %IW4DOOM_EXIT%
