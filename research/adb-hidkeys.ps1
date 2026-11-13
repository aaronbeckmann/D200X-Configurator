# Standalone typing demo. Holds the stock app dead in the boot window, pushes
# d200x-device/hidkeys, starts it detached (it survives the ADB disconnect that
# happens when it switches the gadget to HID), then lets go. The deck then acts
# as a USB keyboard on its own for <Seconds> and reboots itself.
# Its log is written to /data/hidkeys.log on the deck; fetch it on a later
# ADB session with: adb pull /data/hidkeys.log ; adb shell rm /data/hidkeys.log
#
# Usage: powershell -ExecutionPolicy Bypass -File research\adb-hidkeys.ps1 [-Seconds 90]
# Before replugging: quit Ulanzi Studio and focus an empty Notepad window.
param([int]$Seconds = 90, [string]$KbdNode = "/dev/hidg1")

$adb = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Google.PlatformTools*\platform-tools\adb.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
if (-not $adb) { $adb = "adb" }
$bin = Join-Path $PSScriptRoot "..\d200x-device\hidkeys\target\armv7-unknown-linux-musleabihf\release\hidkeys"
if (-not (Test-Path $bin)) { Write-Host "build hidkeys first"; exit 1 }
$outRoot = Join-Path $PSScriptRoot "adb-hidkeys"
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
Start-Sleep -Seconds 8
$state = (& $adb get-state 2>&1)
Write-Host "adb state 8 s after enumeration: $state"
if ($state -ne "device") { Write-Host "ADB went away. Replug and try again."; if (-not $holdProc.HasExited) { $holdProc.Kill() }; exit 1 }

# Fetch the previous run's log first (persistent /data), then launch.
$ts = Get-Date -Format 'yyyyMMdd_HHmmss'
& $adb pull /data/hidkeys.log (Join-Path $outRoot "hidkeys_prev_$ts.log") 2>&1 | Select-Object -Last 1
& $adb shell "ls -la /dev/hidg*" 2>&1 | Out-File (Join-Path $outRoot "hidg_nodes_$ts.txt")
& $adb push $bin /tmp/hidkeys 2>&1 | Select-Object -Last 1
& $adb shell "chmod 755 /tmp/hidkeys; (trap '' HUP; /tmp/hidkeys $Seconds $KbdNode) >/dev/null 2>&1 &"
Start-Sleep -Seconds 1
if (-not $holdProc.HasExited) { $holdProc.Kill() }
Write-Host "hidkeys started on the deck for $Seconds s; the deck will switch to HID and type into the focused window. It reboots itself afterwards."
Write-Host "Finished launching."
