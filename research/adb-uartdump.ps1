# Records the raw MCU serial traffic on /dev/ttyS1 while the stock app is held
# dead (same hold loop as adb-backup.ps1), using the static Rust tool
# d200x-device/uartdump pushed to /tmp. Two passes: 115200 baud (EasyUI.cfg
# value) and 1500000 baud (Ulanzi's TC002 code). Each pass first sends the
# TC002-style "query MCU version" frame FF 55 11 00 01 65 and then records.
# Press keys / turn knobs continuously during both passes. Reboots at the end.
#
# Usage: powershell -ExecutionPolicy Bypass -File research\adb-uartdump.ps1
# then unplug/replug the deck and start pressing ~10 s later, for ~90 s.

$adb = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Google.PlatformTools*\platform-tools\adb.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
if (-not $adb) { $adb = "adb" }
$bin = Join-Path $PSScriptRoot "..\d200x-device\uartdump\target\armv7-unknown-linux-musleabihf\release\uartdump"
if (-not (Test-Path $bin)) { Write-Host "build uartdump first"; exit 1 }
$outRoot = Join-Path $PSScriptRoot "adb-uartdump"
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

& $adb push $bin /tmp/uartdump 2>&1 | Select-Object -Last 1
& $adb shell "chmod 755 /tmp/uartdump; ls -la /tmp/uartdump; /tmp/uartdump" 2>&1 | Out-File (Join-Path $out "smoke.txt")
Write-Host "RECORDING pass 1: 115200 baud, 50 s. Press keys, side buttons, press+turn knobs."
& $adb shell "/tmp/uartdump /dev/ttyS1 115200 50 /tmp/uart115200.bin ff5511000165" 2>&1 | Out-File (Join-Path $out "dump_115200.txt")
Write-Host "RECORDING pass 2: 1500000 baud, 30 s. Keep pressing."
& $adb shell "/tmp/uartdump /dev/ttyS1 1500000 30 /tmp/uart1500000.bin ff5511000165" 2>&1 | Out-File (Join-Path $out "dump_1500000.txt")
& $adb pull /tmp/uart115200.bin (Join-Path $out "uart115200.bin") 2>&1 | Select-Object -Last 1
& $adb pull /tmp/uart1500000.bin (Join-Path $out "uart1500000.bin") 2>&1 | Select-Object -Last 1
Write-Host "Done. Rebooting the deck..."
if (-not $holdProc.HasExited) { $holdProc.Kill() }
& $adb reboot 2>&1
Write-Host "Finished: $out"
