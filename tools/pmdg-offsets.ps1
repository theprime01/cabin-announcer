# Compiles and runs pmdg-offsets.c against the PMDG SDK headers that ship with the aircraft.
# -Community is your MSFS 2024 Community folder (InstalledPackagesPath in UserCfg.opt).
param([Parameter(Mandatory)] [string]$Community)
$vs = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -property installationPath
$ng3 = "$Community\pmdg-aircraft-738\Documentation\SDK"
$t7 = "$Community\pmdg-aircraft-77w\Documentation\SDK"
$out = Join-Path $env:TEMP "pmdg-offsets"
New-Item -ItemType Directory -Force $out | Out-Null
cmd /c "`"$vs\VC\Auxiliary\Build\vcvars64.bat`" >nul && cl /nologo /TP /I`"$ng3`" /I`"$t7`" /Fe`"$out\pmdg-offsets.exe`" /Fo`"$out\\`" `"$PSScriptRoot\pmdg-offsets.c`" >nul"
& "$out\pmdg-offsets.exe"
