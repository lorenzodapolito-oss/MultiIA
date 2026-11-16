@echo off
rem Scarica (o aggiorna) e compila MultiIA. Metti questo file in AmbienteRust.
title Costruisci MultiIA
set "ROOT=%~dp0"
if "%ROOT:~-1%"=="\" set "ROOT=%ROOT:~0,-1%"
rem I pacchetti vanno estratti su un disco NTFS: exFAT/FAT32 (chiavette, dischi USB) non accettano le date
rem 1970 dei pacchetti e cargo fallisce con "failed to set mtime". Toolchain e programmi restano nella cartella.
set "CARGO_HOME=%LOCALAPPDATA%\AmbienteRust-cargo"
if not exist "%CARGO_HOME%" mkdir "%CARGO_HOME%"
set "RUSTUP_HOME=%ROOT%\rust\rustup"
rem Usa direttamente cargo e rustc della toolchain: non serve rustup (che su un PC nuovo prova a riscaricarla).
set "TC="
for /d %%t in ("%ROOT%\rust\rustup\toolchains\*") do if exist "%%t\bin\cargo.exe" set "TC=%%t\bin;"
set "PATH=%TC%%ROOT%\winlibs\mingw64\bin;%ROOT%\rust\cargo\bin;%ROOT%\git\cmd;%ROOT%\git\bin;%PATH%"
set "PROG=%ROOT%\Progetti"
if not exist "%PROG%" mkdir "%PROG%"
cd /d "%PROG%"
taskkill /f /im multiia.exe >nul 2>&1

echo === Scarico o aggiorno MultiIA ===
if exist "%PROG%\MultiIA\.git" (
  git -c "safe.directory=*" -C "%PROG%\MultiIA" fetch origin
  if errorlevel 1 goto fail
  git -c "safe.directory=*" -C "%PROG%\MultiIA" reset --hard origin/main
) else (
  git clone https://github.com/lorenzodapolito-oss/MultiIA "%PROG%\MultiIA"
)
if errorlevel 1 goto fail

echo === Compilo ===
cd /d "%PROG%\MultiIA"
cargo build --release
if errorlevel 1 goto fail

set "OUT=%PROG%\MultiIA\dist\MultiIA-portable"
if exist "%OUT%" rmdir /s /q "%OUT%"
mkdir "%OUT%"
copy /y "target\release\multiia.exe" "%OUT%\MultiIA.exe" >nul
if errorlevel 1 goto fail

echo.
echo FATTO: %OUT%\MultiIA.exe
pause
exit /b 0

:fail
echo.
echo ERRORE: operazione non riuscita. Controlla i messaggi sopra.
pause
exit /b 1
