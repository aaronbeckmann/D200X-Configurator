# Dry run of the installed layout without flashing: stage the shim, deckd and
# font under /tmp (the vendor loader prefers /tmp/EasyUI.cfg, the mechanism
# the FlyThings IDE uses for temporary deploys), then let init start the
# stock service `zkswe`. If the mechanism works, zkgui dlopen()s our shim,
# the shim execs /tmp/bin/deckd, and the deck comes up on deckd as if it
# were installed. Everything is gone again after a replug.
#
# Usage: powershell -ExecutionPolicy Bypass -File research\adb-dryrun.ps1 [-Serial 02D23A045U3672338] [-Uart]
#   -Uart  run deckd with --uart: console UART link to the ESP32 bridge (esp32-bridge/docs/WIRING.md)
param([string]$Serial = "", [switch]$Calibrate, [switch]$Uart)
$adb = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Google.PlatformTools*\platform-tools\adb.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
if (-not $adb) { $adb = "adb" }
$root = Split-Path $PSScriptRoot -Parent
$deckd = Join-Path $root "d200x-device\deckd\target\armv7-unknown-linux-musleabihf\release\deckd"
$shim = Join-Path $root "d200x-device\zkshim\target\armv7-unknown-linux-gnueabihf\release\libzkshim.so"
$cfg = Join-Path $root "d200x-device\image\EasyUI.cfg.tmp"
$font = Join-Path $root "d200x-device\image\font\font.ttf"
$tool = Join-Path $root "d200x-tool\target\debug\d200x-tool.exe"
foreach ($f in @($deckd, $shim, $cfg, $font)) { if (-not (Test-Path $f)) { Write-Host "missing $f"; exit 1 } }
$outRoot = Join-Path $PSScriptRoot "adb-dryrun"
New-Item -ItemType Directory -Force $outRoot | Out-Null
$ts = Get-Date -Format 'yyyyMMdd_HHmmss'
$out = Join-Path $outRoot $ts
New-Item -ItemType Directory -Force $out | Out-Null
& $adb kill-server 2>$null | Out-Null
& $adb start-server 2>$null | Out-Null

# Serial for deckd: the OTP read path is gated by the vendor driver (EPERM), so
# capture the serial from whatever firmware is running now (stock answers with
# the real one) and store it in /data/deckd/serial, which deckd prefers.
if (-not $Serial) {
  $hidNow = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -like 'USB\VID_2207&PID_0019\*' }
  if ($hidNow) {
    $infoNow = (& $tool info 2>$null) -join ""
    if ($infoNow -match '"SerialNumber"\s*:\s*"([A-Za-z0-9]{10,})"' -and $Matches[1] -ne "0123456789ABCDEF") { $Serial = $Matches[1] }
  }
}
Write-Host "serial to store: $(if ($Serial) { $Serial } else { '(none)' })"

$hold = "setprop sys.zkapp.state running; " +
        "while :; do setprop ctl.stop zkswe; " +
        "for p in `$(ps | busybox awk '/zkgui|deckd/ && !/awk/ {print `$1}'); do kill -9 `$p; done; done"
Write-Host "Waiting for the deck's ADB interface (unplug/replug the deck now)..."
$holdProc = Start-Process -FilePath $adb -ArgumentList @("wait-for-device", "shell", "`"$hold`"") -NoNewWindow -PassThru -RedirectStandardOutput (Join-Path $out "hold.log")
& $adb wait-for-device
Start-Sleep -Seconds 8
$state = (& $adb get-state 2>&1)
if ($state -ne "device") { Write-Host "ADB went away. Replug and try again."; if (-not $holdProc.HasExited) { $holdProc.Kill() }; exit 1 }

& $adb pull /data/deckd/deckd.log (Join-Path $out "deckd_prev.log") 2>&1 | Select-Object -Last 1
& $adb shell "mkdir -p /tmp/lib /tmp/bin /data/deckd" | Out-Null
if ($Serial) { & $adb shell "echo -n $Serial > /data/deckd/serial" | Out-Null }
& $adb push $shim /tmp/lib/libzkgui.so 2>&1 | Select-Object -Last 1
if ($Uart) {
  # the shim execs /tmp/bin/deckd without arguments: a wrapper adds --uart (ESP32 bridge link)
  & $adb push $deckd /tmp/bin/deckd.bin 2>&1 | Select-Object -Last 1
  # echo is a shell builtin; this busybox has no printf
  & $adb shell "echo '#!/bin/sh' > /tmp/bin/deckd; echo 'exec /tmp/bin/deckd.bin --uart' >> /tmp/bin/deckd; chmod 755 /tmp/bin/deckd.bin" | Out-Null
} else {
  & $adb push $deckd /tmp/bin/deckd 2>&1 | Select-Object -Last 1
}
& $adb push $font /tmp/font.ttf 2>&1 | Select-Object -Last 1
& $adb push $cfg /tmp/EasyUI.cfg 2>&1 | Select-Object -Last 1
& $adb shell "chmod 755 /tmp/bin/deckd /tmp/lib/libzkgui.so; ls -la /tmp /tmp/lib /tmp/bin; getprop init.svc.zkswe" | Out-File (Join-Path $out "staged.txt")
Write-Host "Staged. Stopping the hold loop and starting the vendor service zkswe..."
if (-not $holdProc.HasExited) { $holdProc.Kill() }
Start-Sleep -Milliseconds 500
& $adb shell "setprop ctl.start zkswe" 2>&1
Start-Sleep -Seconds 2
& $adb shell "ps | busybox awk '/zk|deckd/'; getprop init.svc.zkswe; getprop sys.usb.state; cat /data/deckd/deckd.log" 2>&1 | Out-File (Join-Path $out "after_start.txt")
Get-Content (Join-Path $out "after_start.txt")

Write-Host "Waiting for the HID gadget (deckd switching USB)..."
$deadline = (Get-Date).AddSeconds(40)
do {
  Start-Sleep -Seconds 1
  $hid = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -like 'USB\VID_2207&PID_0019\*' }
} until ($hid -or (Get-Date) -gt $deadline)
if (-not $hid) { Write-Host "HID gadget did not appear (deckd not running via shim?)."; exit 1 }
Start-Sleep -Seconds 3
Write-Host "--- info"
& $tool info 2>&1 | Tee-Object -FilePath (Join-Path $out "info.txt")
Write-Host "--- device log"
& $tool log 2>&1 | Tee-Object -FilePath (Join-Path $out "deckd_log_via_usb.txt") | Select-Object -Last 25
if ($Calibrate) { Write-Host "--- calibration pattern on"; & $tool calibrate on 2>&1 }
Write-Host "Finished: $out  (deckd keeps running until the next replug)"
