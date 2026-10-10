@echo off
rem Builds (first time only, needs Rust: https://rustup.rs) and starts FretWright.
rem   run.bat                 start the app
rem   run.bat cli ...         command line tool (fretwright-cli --help)
setlocal
cd /d "%~dp0"
where cargo >nul 2>nul
if errorlevel 1 (
    echo Rust is needed to build FretWright from source: https://rustup.rs
    echo ^(or download a ready-made release instead^)
    exit /b 1
)
if "%~1"=="cli" goto cli
cargo build --release --quiet -p fw-app || exit /b 1
start "" target\release\fretwright.exe %*
exit /b 0

:cli
rem drop the leading "cli " from the arguments
set "ARGS=%*"
set "ARGS=%ARGS:~4%"
cargo build --release --quiet -p fw-cli || exit /b 1
target\release\fretwright-cli.exe %ARGS%
