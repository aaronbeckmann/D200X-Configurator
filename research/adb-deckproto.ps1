# Runs the on-device Rust prototype (d200x-device/deckproto) from /tmp with
# the stock app held dead: test pattern on /dev/fb0 + live MCU input.
# Usage: powershell -ExecutionPolicy Bypass -File research\adb-deckproto.ps1 [-Seconds 90] [-Rotation 1]
# then unplug/replug the deck. Watch the screen; press keys and turn knobs.
param([int]$Seconds = 90, [int]$Rotation = 1)

$adb = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Google.PlatformTools*\platform-tools\adb.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
if (-not $adb) { $adb = "adb" }
$bin = Join-Path $PSScriptRoot "..\d200x-device\deckproto\target\armv7-unknown-linux-musleabihf\release\deckproto"
if (-not (Test-Path $bin)) { Write-Host "build deckproto first"; exit 1 }
$outRoot = Join-Path $PSScriptRoot "adb-deckproto"
New-Item -ItemType Directory -Force $outRoot | Out-Null
& $adb kill-server 2>$null | Out-Null
& $adb start-server 2>$null | Out-Null

$hold = "setprop sys.zkapp.state running; " +
        "while :; do setprop ctl.stop zkswe; " +
        "for p in `$(ps | busybox awk '/zkgui/ && !/awk/ {print `$1}'); do kill -9 `$p; echo killed `$p; done; done"
$holdLog = Join-Path $outRoot "hold.log"
Write-Host "Waiting for the deck's ADB interface (unplug/replug the deck now)..."
$holdProc = Start-Process -FilePath $adb -ArgumentList @("wait-for-device", "shell", "`"$hold`"") -NoNewWindow -PassThru -RedirectStandardOutput $holdLog
& $adb wait-for-device
$ts = Get-Date -Format 'yyyyMMdd_HHmmss'
$out = Join-Path $outRoot $ts
New-Item -ItemType Directory -Force $out | Out-Null
Start-Sleep -Seconds 8
$state = (& $adb get-state 2>&1)
Write-Host "[$ts] adb state 8 s after enumeration: $state"
if ($state -ne "device") { Write-Host "ADB went away. Replug and try again."; if (-not $holdProc.HasExited) { $holdProc.Kill() }; exit 1 }

& $adb push $bin /tmp/deckproto 2>&1 | Select-Object -Last 1
& $adb shell "ps" | Out-File (Join-Path $out "ps_before.txt")
Write-Host "RUNNING deckproto for $Seconds s (rotation $Rotation). Look at the deck."
& $adb shell "chmod 755 /tmp/deckproto; /tmp/deckproto $Seconds $Rotation" 2>&1 | Out-File (Join-Path $out "deckproto.txt")
Write-Host "Done. Rebooting the deck..."
if (-not $holdProc.HasExited) { $holdProc.Kill() }
& $adb reboot 2>&1
Write-Host "Finished: $out"
