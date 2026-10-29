@echo off
rem ===========================================================================
rem  Timed runner for ???? (keep this file ASCII-only, CRLF line endings).
rem  Usage:
rem    runner.cmd "<path to exe>" [dist_km=2.0] [pace_s_per_km=390] [HH:MM or minutes] [--test]
rem  Examples (arguments only; the script name is whatever you saved it as):
rem    runner.cmd "D:\...\exe" 2.0 390 07:30
rem    runner.cmd "D:\...\exe" 2.0 390 120          :: start 120 min ago
rem    runner.cmd "D:\...\exe" 2.0 390 07:30 --test :: print only, no submit
rem  Logs: logs\yyyyMMdd.log next to this script.
rem ===========================================================================
chcp 65001 >nul
setlocal

set "EXE=%~1"
if "%EXE%"=="" goto usage

set "DIST=%~2"
set "PACE=%~3"
set "WHEN=%~4"
rem --test / --force are accepted in any position, and never left in a value slot
set "TEST="
set "FORCE="
echo %*|findstr /i /c:"--test" >nul && set "TEST=--test"
echo %*|findstr /i /c:"--force" >nul && set "FORCE=--force"
if "%DIST:~0,2%"=="--" set "DIST="
if "%PACE:~0,2%"=="--" set "PACE="
if "%WHEN:~0,2%"=="--" set "WHEN="

if "%DIST%"=="" set "DIST=2.0"
if "%PACE%"=="" set "PACE=390"

rem start time: HH:MM -> --time (inside the school window); digits -> --ago (minutes ago)
set "TARG=--ago 60"
echo %WHEN%|findstr /r "^[0-2][0-9]:[0-5][0-9]$" >nul
if not errorlevel 1 set "TARG=--time %WHEN%"
echo %WHEN%|findstr /r "^[0-9][0-9]*$" >nul
if not errorlevel 1 set "TARG=--ago %WHEN%"
if "%WHEN%"=="" set "TARG=--ago 60"

if not exist "%EXE%" (
    echo [ERROR] exe not found: %EXE%
    exit /b 1
)

if /i "%TEST%"=="--test" (
    echo [dry-run] would run: "%EXE%" run --dist %DIST% --pace %PACE% %TARG% --seed 0
    exit /b 0
)

if not exist "%~dp0logs" mkdir "%~dp0logs" >nul 2>&1
for /f "delims=" %%i in ('powershell -NoProfile -Command "Get-Date -Format 'yyyy-MM-dd HH:mm:ss'"') do set "STAMP=%%i"
for /f "delims=" %%i in ('powershell -NoProfile -Command "Get-Date -Format yyyyMMdd"') do set "DAY=%%i"
set "LOG=%~dp0logs\%DAY%.log"

rem same-day guard: one successful record per day is enough
if /i not "%FORCE%"=="--force" (
    if exist "%LOG%" findstr /c:"exit 0" "%LOG%" >nul && (
        echo [skip] already succeeded today, use --force to run again>>"%LOG%"
        exit /b 0
    )
)

echo.>>"%LOG%"
echo ===== %STAMP% start: --dist %DIST% --pace %PACE% %TARG% =====>>"%LOG%"
rem NOTE: keep a space before the redirect - "0>>file" would be read as fd 0
"%EXE%" run --dist %DIST% --pace %PACE% %TARG% --seed 0 >>"%LOG%" 2>&1
set "CODE=%ERRORLEVEL%"
echo ----- %STAMP% exit %CODE% ----->>"%LOG%"
exit /b %CODE%

:usage
echo Usage: runner.cmd "path to exe" [dist_km] [pace_s_per_km] [HH:MM or minutes] [--test]
exit /b 2
