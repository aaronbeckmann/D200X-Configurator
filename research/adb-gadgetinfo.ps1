# Diagnostic: in the boot ADB window (app held dead), fetch /data/hidkeys.log
# from the previous hidkeys run and dump everything about the USB gadget and
# its HID functions from sysfs, then reboot.
$adb = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Google.PlatformTools*\platform-tools\adb.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
if (-not $adb) { $adb = "adb" }
$outRoot = Join-Path $PSScriptRoot "adb-gadgetinfo"
New-Item -ItemType Directory -Force $outRoot | Out-Null
& $adb kill-server 2>$null | Out-Null
& $adb start-server 2>$null | Out-Null
$hold = "setprop sys.zkapp.state running; " +
        "while :; do setprop ctl.stop zkswe; " +
        "for p in `$(ps | busybox awk '/zkgui/ && !/awk/ {print `$1}'); do kill -9 `$p; done; done"
Write-Host "Waiting for the deck's ADB interface (unplug/replug the deck now)..."
$holdProc = Start-Process -FilePath $adb -ArgumentList @("wait-for-device", "shell", "`"$hold`"") -NoNewWindow -PassThru -RedirectStandardOutput (Join-Path $outRoot "hold.log")
& $adb wait-for-device
Start-Sleep -Seconds 8
$state = (& $adb get-state 2>&1)
if ($state -ne "device") { Write-Host "ADB went away. Replug and try again."; if (-not $holdProc.HasExited) { $holdProc.Kill() }; exit 1 }
$ts = Get-Date -Format 'yyyyMMdd_HHmmss'
$out = Join-Path $outRoot $ts
New-Item -ItemType Directory -Force $out | Out-Null
& $adb pull /data/hidkeys.log (Join-Path $out "hidkeys.log") 2>&1 | Select-Object -Last 1
& $adb shell "rm /data/hidkeys.log" | Out-Null
function Dump($name, $cmd) { $f = Join-Path $out "$name.txt"; "`$ $cmd" | Out-File $f; & $adb shell $cmd 2>&1 | Out-File -Append $f }
Dump "zkswe_usb"  "ls -laR /sys/devices/virtual/zkswe_usb; for f in /sys/devices/virtual/zkswe_usb/zkswe0/*; do echo == `$f; cat `$f 2>&1; done"
Dump "f_hid"      "for f in /sys/devices/virtual/zkswe_usb/zkswe0/f_hid/* /sys/devices/virtual/zkswe_usb/zkswe0/f_adb/*; do echo == `$f; cat `$f 2>&1; done"
Dump "hidg_class" "ls -laR /sys/class/hidg /sys/devices/virtual/hidg /sys/class/misc 2>&1; for d in /sys/class/hidg/*; do echo == `$d; cat `$d/dev `$d/uevent 2>&1; ls -la `$d/ 2>&1; done"
Dump "dev_nodes"  "ls -la /dev/hidg* /dev/android_adb 2>&1; cat /proc/devices"
Dump "udc"        "ls -laR /sys/class/udc 2>&1; cat /sys/class/udc/*/state 2>&1; ls -la /sys/kernel/config 2>&1; ls -laR /sys/kernel/config/usb_gadget 2>&1"
Dump "modules"    "cat /proc/modules; ls -la /config/modules"
Dump "hid_in_lib" "ls -la /lib | busybox awk '/hid|usb|gadget/'"
Write-Host "Done. Rebooting..."
if (-not $holdProc.HasExited) { $holdProc.Kill() }
& $adb reboot 2>&1
Write-Host "Finished: $out"
