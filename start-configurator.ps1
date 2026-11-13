# Start the D200X Configurator (release build). Builds it first when the exe is missing
# or older than the sources. Usage: .\start-configurator.ps1 [-Build]
param([switch]$Build)

$root = $PSScriptRoot
$crate = Join-Path $root "d200x-configurator"
$exe = Join-Path $crate "target\release\d200x-configurator.exe"

$needBuild = $Build -or -not (Test-Path $exe)
if (-not $needBuild) {
    $exeTime = (Get-Item $exe).LastWriteTime
    $srcDirs = @("d200x-configurator\src", "d200x-core\src", "deck-config\src", "deck-icons\src") | ForEach-Object { Join-Path $root $_ }
    $newest = Get-ChildItem $srcDirs -Recurse -Include *.rs, Cargo.toml | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($newest -and $newest.LastWriteTime -gt $exeTime) { $needBuild = $true }
}

if ($needBuild) {
    Write-Host "Building the configurator (cargo build --release)..."
    Push-Location $crate
    try {
        & cargo build --release
        if ($LASTEXITCODE -ne 0) { Write-Host "build failed"; exit 1 }
    } finally {
        Pop-Location
    }
}

Start-Process -FilePath $exe -WorkingDirectory $root
Write-Host "started $exe"
