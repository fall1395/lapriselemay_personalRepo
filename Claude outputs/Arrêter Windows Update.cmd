@echo off
chcp 65001 >nul
title Arrêter Windows Update
rem Arrête Windows Update le temps d'une partie : son orchestrateur, Windows Update lui-même,
rem et BITS et l'Optimisation de livraison, qui font ses téléchargements.
rem Rien n'est désactivé : « Relancer Windows Update.cmd », ou un redémarrage du PC, remet tout
rem en marche.

rem Les droits administrateur sont nécessaires : le script se relance avec, via l'invite de Windows.
fltmc >nul 2>&1 || (
  set "WU_SCRIPT=%~f0"
  powershell -NoProfile -Command "Start-Process -FilePath $env:WU_SCRIPT -Verb RunAs"
  exit /b
)

set ENCORE=0
echo Arrêt de Windows Update...
echo.
call :arreter UsoSvc "Orchestrateur des mises à jour"
call :arreter wuauserv "Windows Update"
call :arreter BITS "Transfert intelligent en arrière-plan (BITS)"
call :arreter DoSvc "Optimisation de livraison"
echo.
if %ENCORE% gtr 0 (
  echo Un service tourne encore : relance ce script dans un instant.
) else (
  echo C'est fait. Si Windows en relance un pendant la partie, relance ce script.
)
echo Après ta partie : « Relancer Windows Update.cmd ».
echo.
pause
exit /b

:arreter
net stop %1 /y >nul 2>&1
sc query %1 | findstr /C:"RUNNING" >nul
if errorlevel 1 (set "ETAT=arrêté") else (set "ETAT=encore en marche" & set /a ENCORE+=1)
echo   %~2 : %ETAT%
exit /b
