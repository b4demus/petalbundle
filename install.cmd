@echo off
chcp 65001 >nul
setlocal EnableDelayedExpansion
rem Installs (replaces) every plugin from dist\ into the system VST3 folder.
rem   install.cmd        install, asks for admin rights
rem   install.cmd /dry   only show what would be replaced

rem Release zip: plugins in VST3\ next to this file; source checkout: dist\.
set "SRC=%~dp0dist"
if exist "%~dp0VST3\*.vst3" set "SRC=%~dp0VST3"
set "DST=%CommonProgramFiles%\VST3"
set "DRY="
if /i "%~1"=="/dry" set "DRY=1"

if not exist "%SRC%\*.vst3" (
    echo В "%SRC%" нет плагинов. Сначала собери: powershell -ExecutionPolicy Bypass -File build.ps1
    goto :done
)

if not defined DRY (
    net session >nul 2>&1
    if errorlevel 1 (
        echo Нужны права администратора, запрашиваю...
        powershell -NoProfile -Command "Start-Process -FilePath '%~f0' -Verb RunAs"
        exit /b
    )
)

:check_fl
set "FL_RUNNING="
for %%E in (FL64.exe FL.exe) do (
    tasklist /FI "IMAGENAME eq %%E" 2>nul | "%SystemRoot%\System32\find.exe" /I "%%E" >nul && set "FL_RUNNING=1"
)
if defined FL_RUNNING (
    if defined DRY (
        echo [dry] FL Studio сейчас запущен — при установке нужно будет его закрыть.
    ) else (
        echo FL Studio запущен: пока он открыт, файлы плагинов заняты.
        echo Закрой FL и нажми любую клавишу...
        pause >nul
        goto :check_fl
    )
)

echo Папка установки: %DST%
echo.
rem Botanica was renamed to Petal Granulizer (same VST3 ID): remove the old copy.
if exist "%DST%\Botanica.vst3" (
    if defined DRY (
        echo [dry] Botanica.vst3  ^(старая версия, будет удалена^)
    ) else (
        rmdir /s /q "%DST%\Botanica.vst3" && echo   удалена старая Botanica.vst3 ^(теперь Petal Granulizer^)
    )
)
set "FAILED=0"
for /d %%P in ("%SRC%\*.vst3") do (
    if exist "%DST%\%%~nxP" (set "ACTION=замена") else (set "ACTION=новый")
    if defined DRY (
        echo [dry] %%~nxP  ^(!ACTION!^)
    ) else (
        rem /MIR makes the target an exact copy, so stale files are removed too.
        robocopy "%%P" "%DST%\%%~nxP" /MIR /R:2 /W:1 /NJH /NJS /NFL /NDL /NP >nul
        if errorlevel 8 (
            echo   ОШИБКА: %%~nxP  ^(файл занят или нет доступа^)
            set "FAILED=1"
        ) else (
            echo   готово: %%~nxP  ^(!ACTION!^)
        )
    )
)

echo.
if "%FAILED%"=="1" (
    echo Не всё установилось. Проверь, что FL закрыт, и запусти ещё раз.
) else if not defined DRY (
    echo Всё установлено. Открывай FL — новые плагины найдутся через Manage plugins.
)

:done
echo.
pause
