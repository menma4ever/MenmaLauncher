@echo off
REM Start server: Menma server + playit.gg tunnel together. Host PC only.
setlocal
set "ROOT=%~dp0.."

if not exist "%ROOT%\server\start.bat" (
  echo No Menma server on this PC.
  exit /b 1
)

REM Don't start twice: is something already listening on the game port?
netstat -ano | findstr /r /c:":25565 .*LISTENING" >nul
if not errorlevel 1 (
  echo The server is already running on port 25565.
  exit /b 0
)

REM playit runs in its own window (standalone mode, no Windows service).
tasklist /fi "imagename eq playit.exe" | find /i "playit.exe" >nul
if errorlevel 1 (
  start "playit.gg tunnel" /d "%ROOT%\tools\playit" "%ROOT%\tools\playit\playit.exe"
)

start "Menma server" /d "%ROOT%\server" cmd /c "%ROOT%\server\start.bat"
echo Started the Menma server and the playit tunnel.
