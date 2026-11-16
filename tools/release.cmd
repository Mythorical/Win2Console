@echo off
rem Builds a Win2Console release in the VM: Setup.exe (installer), Portable.zip and the update packages.
rem Usage (from tools/vm.sh): release.cmd <version>      e.g. release.cmd 0.2.0
rem Output: C:\cs-target\releases\  (published to GitHub by tools/publish.sh on the host)
setlocal
if "%~1"=="" (echo usage: release.cmd ^<version^> & exit /b 2)
set VERSION=%~1
set PACK=C:\cs-target\pack
set OUT=C:\cs-target\releases
cd /d C:\cs-src || exit /b 1

echo == Building Win2Console (release)
cargo build --release -p console-shell || exit /b 1
echo == Building the Playnite bridge
dotnet build playnite-extension -c Release -o C:\cs-target\bridge -v quiet || exit /b 1

echo == Assembling %PACK%
if exist %PACK% rmdir /s /q %PACK%
mkdir %PACK%\playnite-bridge
copy /y C:\cs-target\release\Win2Console.exe %PACK%\ >nul || exit /b 1
copy /y C:\cs-target\release\discord_partner_sdk.dll %PACK%\ >nul || exit /b 1
copy /y C:\cs-target\bridge\Win2ConsoleBridge.dll %PACK%\playnite-bridge\ >nul || exit /b 1
copy /y playnite-extension\extension.yaml %PACK%\playnite-bridge\ >nul || exit /b 1

echo == Fetching the previous release (for small delta updates; fine to fail on the first release)
"%USERPROFILE%\.dotnet\tools\vpk.exe" download github --repoUrl https://github.com/Mythorical/Win2Console --outputDir %OUT%

echo == Packing %VERSION%
"%USERPROFILE%\.dotnet\tools\vpk.exe" pack --packId Win2Console --packVersion %VERSION% --packDir %PACK% ^
  --mainExe Win2Console.exe --packTitle Win2Console --packAuthors Mythorical --outputDir %OUT% || exit /b 1
echo == Done: %OUT%
dir /b %OUT%
