@echo off
setlocal
rem Builds the latest release exe, shows how long the build took, then runs it.
rem Arguments are passed on to cuota.exe, e.g.  build-run.bat --demo

cd /d "%~dp0"

rem Explorer runs a double-clicked script as  cmd /c ""script" "  (note the trailing space).
rem In that case, wait at the end so the window doesn't close before the output can be read.
set "CMDLINE=%cmdcmdline:"=%"
if "%CMDLINE:~-1%"==" " set "DOUBLE_CLICKED=1"

rem A running cuota.exe locks the file, so the build couldn't replace it.
taskkill /im cuota.exe /f >nul 2>&1

call :now START
cargo build --release
set "BUILD_RESULT=%ERRORLEVEL%"
call :now END

set /a ELAPSED=END-START
rem The clock passed midnight during the build.
if %ELAPSED% lss 0 set /a ELAPSED+=8640000
set /a MINUTES=ELAPSED/6000, SECONDS=ELAPSED%%6000/100, HUNDREDTHS=ELAPSED%%100
if %HUNDREDTHS% lss 10 set "HUNDREDTHS=0%HUNDREDTHS%"
set "BUILD_TIME=%MINUTES%m %SECONDS%.%HUNDREDTHS%s"

echo.
if not "%BUILD_RESULT%"=="0" (
    echo Build FAILED after %BUILD_TIME%, so cuota.exe was not started.
    goto done
)
echo Build time: %BUILD_TIME%
echo Starting cuota.exe %*
start "" "target\release\cuota.exe" %*

:done
if defined DOUBLE_CLICKED pause
exit /b %BUILD_RESULT%

rem Stores the current time of day in %1, in hundredths of a second.
rem The leading 1 and -100 keep hours like 08 from being read as octal.
:now
set "T=%TIME: =0%"
set /a %1=((1%T:~0,2%-100)*3600 + (1%T:~3,2%-100)*60 + 1%T:~6,2%-100)*100 + 1%T:~9,2%-100
exit /b
