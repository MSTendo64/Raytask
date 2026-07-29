@echo off
setlocal

if "%RAYTASK_REGISTRY_HOST%"=="" set RAYTASK_REGISTRY_HOST=0.0.0.0
if "%RAYTASK_REGISTRY_PORT%"=="" set RAYTASK_REGISTRY_PORT=8080
if "%RAYTASK_REGISTRY_APP_ROOT%"=="" set RAYTASK_REGISTRY_APP_ROOT=deploy/registry-windows-server/data

if exist target\release\raytask.exe (
  target\release\raytask.exe run apps/registry/main.rt
) else (
  cargo run -- run apps/registry/main.rt
)
