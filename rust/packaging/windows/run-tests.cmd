@echo off
rem GCarve self-check on this PC: starts gcarve.exe with samples\showcase.tlp, saves a screenshot after
rem the first frames have rendered, exits. Writes test-results.txt + test-shot.png here. Send both back.
setlocal
set "HERE=%~dp0"
cd /d "%HERE%"
set "OUT=%HERE%test-results.txt"
echo GCarve self-check %DATE% %TIME% > "%OUT%"
ver >> "%OUT%"
echo Folder: %HERE% >> "%OUT%"
echo PROCESSOR_ARCHITECTURE=%PROCESSOR_ARCHITECTURE% %PROCESSOR_ARCHITEW6432% >> "%OUT%"
del /q "%HERE%test-shot.png" 2>nul
echo Starting GCarve (closes by itself)...
set "GCARVE_SHOT=%HERE%test-shot.png"
set "GCARVE_SHOT_FRAME=20"
start "" /wait "%HERE%gcarve.exe" "%HERE%samples\showcase.tlp"
echo gcarve.exe exit code: %ERRORLEVEL% >> "%OUT%"
set "GCARVE_SHOT="
if exist "%HERE%test-shot.png" (
  echo PASS window opened, OpenGL rendered, screenshot saved >> "%OUT%"
  echo RESULT: PASSED >> "%OUT%"
) else (
  echo FAIL no screenshot: the window did not open or OpenGL failed >> "%OUT%"
  echo RESULT: FAILED >> "%OUT%"
)
type "%OUT%"
start "" notepad "%OUT%"
