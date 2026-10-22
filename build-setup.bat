@echo off
setlocal
rem Builds the release Cuota, then the setup that carries it: target\dist\release\cuota-setup.exe,
rem published as CuotaSetup.exe. Its own target folder leaves a running Cuota alone.

cd /d "%~dp0"

rem Explorer runs a double-clicked script as  cmd /c ""script" "  (note the trailing space).
rem In that case, wait at the end so the window doesn't close before the output can be read.
set "CMDLINE=%cmdcmdline:"=%"
if "%CMDLINE:~-1%"==" " set "DOUBLE_CLICKED=1"

set "TARGET=target\dist"
cargo build --release --target-dir "%TARGET%" && cargo build --release -p cuota-setup --target-dir "%TARGET%"
set "BUILD_RESULT=%ERRORLEVEL%"

echo.
if "%BUILD_RESULT%"=="0" (
    echo Built %TARGET%\release\cuota-setup.exe
) else (
    echo Build FAILED.
)
if defined DOUBLE_CLICKED pause
exit /b %BUILD_RESULT%
