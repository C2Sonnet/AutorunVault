@echo off
chcp 65001 >nul
cd /d "%~dp0"
cargo build --release
if errorlevel 1 (
  echo BUILD FAILED
  pause
  exit /b 1
)
echo Build success: target\release\kiseki_autorun_vault.exe
pause
