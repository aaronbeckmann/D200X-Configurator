# Builds the Windows installer:
#   dist\D200X-Configurator-<version>.msi        (per-user MSI)
#   dist\D200X-Configurator-<version>-setup.exe  (setup program wrapping the MSI)
# Needs Rust and the WiX toolset 5 (dotnet tool install --global wix --version 5.0.2).
#
# The programs are compiled into dist\target, so running development builds in
# the crates' own target\release folders are not touched.

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# never change these: they let new versions replace old ones
$upgradeCode = 'B9142E70-E400-4667-A4A7-3B47F56B81F6'
$bundleUpgradeCode = 'D6B39170-00D6-4918-9EF5-CED4603DF3E1'

$version = (Select-String -Path d200x-configurator\Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "D200X Configurator $version" -ForegroundColor Cyan

$dist = Join-Path $root 'dist'
$targetDir = Join-Path $dist 'target'
foreach ($crate in 'd200x-configurator', 'd200x-agent', 'd200x-tool') {
    Write-Host "building $crate..."
    cargo build --release --manifest-path "$crate\Cargo.toml" --target-dir $targetDir
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed for $crate" }
}
$bin = Join-Path $targetDir 'release'

if (-not (Test-Path 'assets\mdi\svg')) { throw 'assets\mdi\svg missing' }
if (-not (Test-Path 'd200x-agent\bridge\node_modules')) {
    Write-Host 'installing the Node bridge dependencies (npm ci)...'
    Push-Location d200x-agent\bridge
    try { npm ci --omit=dev; if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' } } finally { Pop-Location }
}

# WiX extensions: the NuGet packages may ship the DLL under a newer name than
# WiX 5 expects (then listed as "damaged"), so pass the DLL paths directly.
$wixVersion = ((wix --version) -split '\+')[0]
function Get-WixExtension([string]$id) {
    $dir = Join-Path $env:USERPROFILE ".wix\extensions\$id\$wixVersion"
    if (-not (Test-Path $dir)) {
        wix extension add -g "$id/$wixVersion"
        if ($LASTEXITCODE -ne 0) { throw "could not add $id" }
    }
    $dll = Get-ChildItem $dir -Recurse -Filter '*.wixext.dll' | Select-Object -First 1 -ExpandProperty FullName
    if (-not $dll) { throw "no extension DLL found in $dir" }
    $dll
}
$utilExt = Get-WixExtension 'WixToolset.Util.wixext'
$balExt = Get-WixExtension 'WixToolset.Bal.wixext'

New-Item -ItemType Directory -Force $dist | Out-Null
$msi = Join-Path $dist "D200X-Configurator-$version.msi"
$exe = Join-Path $dist "D200X-Configurator-$version-setup.exe"

wix build installer\D200X.wxs -arch x64 -ext $utilExt `
    -d "Version=$version" -d "UpgradeCode=$upgradeCode" -d "BinDir=$bin" -d "RootDir=$root" -o $msi
if ($LASTEXITCODE -ne 0) { throw 'MSI build failed' }

wix build installer\Bundle.wxs -arch x64 -ext $balExt `
    -d "Version=$version" -d "BundleUpgradeCode=$bundleUpgradeCode" -d "MsiPath=$msi" -o $exe
if ($LASTEXITCODE -ne 0) { throw 'setup.exe build failed' }

Get-ChildItem $dist -Filter "D200X-Configurator-$version*" | Where-Object { $_.Extension -in '.msi', '.exe' } |
    Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 1) } }
