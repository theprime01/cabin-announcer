# Builds the release zip: cabin-announcer.exe, SimConnect.dll, README and LICENSE.
param(
    [string]$Out = (Join-Path (Split-Path $PSScriptRoot) "target\dist"),
    [string]$SimConnect
)
$root = Split-Path $PSScriptRoot
$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$stage = Join-Path $Out "Cabin Announcer"
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
& "$PSScriptRoot\package.ps1" -Dest $stage -SimConnect $SimConnect
if (-not $?) { throw "packaging failed" }
Copy-Item "$root\LICENSE" $stage -Force
$zip = Join-Path $Out "cabin-announcer-$version-windows-x64.zip"
Compress-Archive -Path $stage -DestinationPath $zip -Force
"Release zip: $zip"
