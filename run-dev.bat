@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\LaunchDevCmd.bat"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
npm run dev
