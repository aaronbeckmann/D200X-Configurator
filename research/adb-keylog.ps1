# Captures the MCU serial frames for keys, side buttons and knobs by letting the
# stock app run while keeping ADB available.
#
# How: in the boot-time ADB window, start a loop on the deck that ignores SIGHUP
# (so it survives the ADB disconnect) and flips sys.usb.config back to "adb"
# whenever the app has switched it to "hid". The app keeps running and logs
# every MCU frame it receives ("<== frame ..."). We then wait for ADB to come
# back, clear logcat, and record logcat while you press every key, both side
# buttons, and press + turn each knob. Finally the deck is rebooted.
#
# Usage: powershell -ExecutionPolicy Bypass -File research\adb-keylog.ps1
# then unplug/replug the deck, wait for "RECORDING", press things for ~60 s.

$adb = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Google.PlatformTools*\platform-tools\adb.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
if (-not $adb) { $adb = "adb" }
$outRoot = Join-Path $PSScriptRoot "adb-keylog"
New-Item -ItemType Directory -Force $outRoot | Out-Null
& $adb kill-server 2>$null | Out-Null
& $adb start-server 2>$null | Out-Null

$flip = "(trap '' HUP; while :; do if [ `"`$(getprop sys.usb.state)`" = hid ]; then setprop sys.usb.config adb; fi; done) >/dev/null 2>&1 &"
Write-Host "Waiting for the deck's ADB interface (unplug/replug the deck now)..."
& $adb wait-for-device shell $flip
Write-Host "flip-back loop installed; waiting for the app to switch USB and for ADB to return..."
Start-Sleep -Seconds 4
$deadline = (Get-Date).AddSeconds(60)
do {
  Start-Sleep -Milliseconds 500
  $state = (& $adb get-state 2>&1)
} until ($state -eq "device" -or (Get-Date) -gt $deadline)
if ($state -ne "device") { Write-Host "ADB did not come back. Replug and try again."; exit 1 }
Start-Sleep -Seconds 2
$ts = Get-Date -Format 'yyyyMMdd_HHmmss'
$out = Join-Path $outRoot $ts
New-Item -ItemType Directory -Force $out | Out-Null
& $adb shell "ps; getprop sys.usb.state; getprop init.svc.zkswe" | Out-File (Join-Path $out "state.txt")
& $adb shell "logcat -c" | Out-Null
Write-Host ""
Write-Host "RECORDING for 75 s: press every key, both side buttons, press and turn each knob (left and right)."
$logFile = Join-Path $out "logcat.txt"
$p = Start-Process -FilePath $adb -ArgumentList @("shell", "logcat -v time") -NoNewWindow -PassThru -RedirectStandardOutput $logFile
Start-Sleep -Seconds 75
if (-not $p.HasExited) { $p.Kill() }
Write-Host "Recorded $((Get-Content $logFile | Measure-Object -Line).Lines) log lines -> $logFile"
& $adb shell "logcat -d -v time" | Out-File (Join-Path $out "logcat_full.txt")
Write-Host "Rebooting the deck to restore normal operation..."
& $adb reboot 2>&1
Write-Host "Finished: $out"
