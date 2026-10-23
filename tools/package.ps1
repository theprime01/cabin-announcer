# Builds a release and copies it, with SimConnect.dll, to -Dest.
param(
    [Parameter(Mandatory)] [string]$Dest,
    [string]$SimConnect
)
$root = Split-Path $PSScriptRoot
# cargo reports progress on stderr, which PowerShell 5.1 turns into errors under "Stop".
cmd /c "cd /d `"$root`" && cargo build --release"
if ($LASTEXITCODE) { throw "cargo build failed" }
$ErrorActionPreference = "Stop"

if (-not $SimConnect) {
    $SimConnect = @(
        "$root\SimConnect.dll",
        "$env:MSFS2024_SDK\SimConnect SDK\lib\SimConnect.dll",
        "C:\MSFS 2024 SDK\SimConnect SDK\lib\SimConnect.dll"
    ) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (-not $SimConnect) { throw "SimConnect.dll not found; pass -SimConnect <path>" }

New-Item -ItemType Directory -Force $Dest | Out-Null
Copy-Item "$root\target\release\cabin-announcer.exe" $Dest -Force
Copy-Item $SimConnect (Join-Path $Dest "SimConnect.dll") -Force
Copy-Item "$root\README.md" $Dest -Force
"Installed to $Dest (SimConnect.dll from $SimConnect)"
