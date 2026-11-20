@echo off
chcp 65001 >nul
title Relancer Windows Update
rem Remet en marche ce qu'« Arrêter Windows Update.cmd » a arrêté : les mises à jour reprennent.

rem Les droits administrateur sont nécessaires : le script se relance avec, via l'invite de Windows.
fltmc >nul 2>&1 || (
  set "WU_SCRIPT=%~f0"
  powershell -NoProfile -Command "Start-Process -FilePath $env:WU_SCRIPT -Verb RunAs"
  exit /b
)

set ECHECS=0
echo Relance de Windows Update...
echo.
call :relancer BITS "Transfert intelligent en arrière-plan (BITS)"
call :relancer DoSvc "Optimisation de livraison"
call :relancer wuauserv "Windows Update"
call :relancer UsoSvc "Orchestrateur des mises à jour"
echo.
if %ECHECS% gtr 0 (
  echo Un service n'a pas démarré : un redémarrage du PC le relancera.
) else (
  echo C'est fait : les mises à jour reprennent.
)
echo.
pause
exit /b

:relancer
net start %1 >nul 2>&1
sc query %1 | findstr /C:"RUNNING" >nul
if errorlevel 1 (set "ETAT=pas démarré" & set /a ECHECS+=1) else (set "ETAT=en marche")
echo   %~2 : %ETAT%
exit /b
