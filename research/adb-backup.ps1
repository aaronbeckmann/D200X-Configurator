# v2: waits for the deck's boot-time ADB window, immediately stops the
# vendor app services so the USB gadget stays in ADB mode, then takes a
# full read-only backup (all 8 MTD partitions, root filesystem files,
# properties, logcat) into research/adb-backup/<timestamp>/ and reboots
# the deck back to normal. Nothing is written to the deck's flash.
#
# Usage: powershell -ExecutionPolicy Bypass -File research\adb-backup.ps1
# then unplug/replug the deck.

$adb = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Google.PlatformTools*\platform-tools\adb.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
if (-not $adb) { $adb = "adb" }
$outRoot = Join-Path $PSScriptRoot "adb-backup"
New-Item -ItemType Directory -Force $outRoot | Out-Null
& $adb kill-server 2>$null | Out-Null
& $adb start-server 2>$null | Out-Null

Write-Host "Waiting for the deck's ADB interface (unplug/replug the deck now)..."
# Blocks until a device appears, then runs the stop sequence as fast as possible.
# zkdaemon is stopped first so it cannot trigger its recovery reflash while
# the app is down; zkswe is the service that runs zkgui (the deck app), which
# is what switches the USB gadget from adb to HID.
# This busybox has no `stop` binary and `setprop ctl.stop` was ignored (runs 1
# and 2). zkgui is what runs `setprop sys.usb.config hid` a few seconds after
# boot, and init restarts the zkswe service whenever it dies. So: kill zkgui
# by PID in a background loop on the deck for the duration of the backup, keep
# sys.zkapp.state=running so zkdaemon's recovery check stays satisfied, and
# force-stop the watchdog node the app uses, if it exists. `adb reboot` at the
# end restores everything.
# Run 3 showed the app (zkgui) is not even running yet when ADB first appears;
# it starts a few seconds later and switches USB immediately. A loop started
# with `&` dies when the adb shell command returns. So the suppression loop
# runs in its own adb shell session that this script keeps open for the whole
# backup: it keeps issuing ctl.stop for the service and kills any zkgui PID.
$hold = "setprop sys.zkapp.state running; " +
        "while :; do setprop ctl.stop zkswe; " +
        "for p in `$(ps | busybox awk '/zkgui/ && !/awk/ {print `$1}'); do kill -9 `$p; echo killed `$p; done; done"
$holdLog = Join-Path $outRoot "hold.log"
$holdProc = Start-Process -FilePath $adb -ArgumentList @("wait-for-device", "shell", "`"$hold`"") -NoNewWindow -PassThru -RedirectStandardOutput $holdLog
# Wait for the device, then give the boot sequence time to pass the moment
# where the app would normally have switched the gadget.
& $adb wait-for-device
$ts = Get-Date -Format 'yyyyMMdd_HHmmss'
$out = Join-Path $outRoot $ts
New-Item -ItemType Directory -Force $out | Out-Null
Start-Sleep -Seconds 8
$state = (& $adb get-state 2>&1)
Write-Host "[$ts] adb state 8 s after enumeration: $state -> $out"
Write-Host "hold loop says: $((Get-Content $holdLog -ErrorAction SilentlyContinue | Select-Object -Last 3) -join ' | ')"
if ($state -ne "device") {
  Write-Host "ADB went away despite the hold loop. Replug and try again."
  if (-not $holdProc.HasExited) { $holdProc.Kill() }
  exit 1
}

function Dump($name, $cmd) {
  $f = Join-Path $out "$name.txt"
  "`$ $cmd" | Out-File $f
  & $adb shell $cmd 2>&1 | Out-File -Append $f
}

Dump "props"       "getprop"
Dump "services"    "getprop init.svc.zkswe; getprop init.svc.zkdaemon; getprop init.svc.adbd; getprop sys.usb.state; ps"
Dump "ps"          "ps"
Dump "cmdline"     "cat /proc/cmdline; cat /proc/version"
Dump "mtd"         "cat /proc/mtd"
Dump "mounts"      "cat /proc/mounts"
Dump "fb"          "cat /sys/class/graphics/fb0/virtual_size /sys/class/graphics/fb0/bits_per_pixel /sys/class/graphics/fb0/stride /sys/class/graphics/fb0/modes /sys/class/graphics/fb0/name 2>&1"
Dump "dev"         "ls -laR /dev"
Dump "ls_root"     "ls -laR /bin /sbin /etc /lib /system /config /data /misc /extres /overlay /mnt 2>&1"
Dump "sys_soc"     "ls -laR /sys/devices/soc0 2>&1"
Dump "sys_class"   "ls -la /sys/class /sys/class/* 2>&1"
Dump "sys_usb"     "ls -laR /sys/class/android_usb /sys/class/zkswe_usb /sys/devices/soc0/soc/soc:usbotg 2>&1; cat /sys/class/android_usb/android0/* 2>&1"
Dump "tty"         "ls -la /dev/ttyS* /sys/class/tty 2>&1; cat /proc/tty/driver/* 2>&1"
Dump "interrupts"  "cat /proc/interrupts; cat /proc/iomem"
Dump "gpio"        "cat /sys/kernel/debug/gpio 2>&1; ls /sys/class/gpio 2>&1"
Dump "kmsg"        "cat /sys/kernel/debug/dmesg 2>&1; cat /proc/last_kmsg 2>&1"
Dump "logcat"      "logcat -d 2>&1"
Dump "data"        "ls -laR /data; cat /data/preferences.json; cat /data/property/* 2>&1"
Dump "config_ini"  "cat /config/board.ini /config/PQConfig.ini /config/mmap.ini; ls -la /config/model /config/modules"
Dump "init_rc"     "cat /init.rc /etc/init.rc /init*.rc /etc/*.rc 2>&1; ls -la / 2>&1"
Dump "storage"     "ls -la /mnt/storage; cat /mnt/storage/binversion"

Write-Host "Pulling root filesystem directories..."
# Already captured completely in run 20261029_201632; keep only the small/variable ones.
foreach ($d in @("/etc","/data")) {
  $dst = Join-Path $out ("fs" + $d.Replace("/","\"))
  New-Item -ItemType Directory -Force $dst | Out-Null
  & $adb pull $d $dst 2>&1 | Select-Object -Last 1
}
& $adb pull /init.rc $out 2>$null | Out-Null
& $adb pull /init $out 2>$null | Out-Null

Write-Host "Dumping MTD partitions (32 MiB SPI NOR)..."
$names = @("BOOT0","KERNEL","rootfs","res","config","MISC","data","UDISK")
for ($i = 0; $i -lt 8; $i++) {
  $f = Join-Path $out ("mtd{0}_{1}.bin" -f $i, $names[$i])
  # This adbd predates exec-out, so stage each partition on tmpfs and pull it.
  & $adb shell "cp /dev/block/mtdblock$i /tmp/mtd$i.bin" 2>&1 | Out-Null
  & $adb pull "/tmp/mtd$i.bin" $f 2>&1 | Select-Object -Last 1
  & $adb shell "rm /tmp/mtd$i.bin" 2>&1 | Out-Null
  $len = if (Test-Path $f) { (Get-Item $f).Length } else { 0 }
  Write-Host ("  mtd{0} {1}: {2} bytes" -f $i, $names[$i], $len)
}
Dump "md5" "for i in 0 1 2 3 4 5 6 7; do md5sum /dev/block/mtdblock`$i 2>&1; done"

Write-Host "Backup done. Rebooting the deck to restore normal operation..."
if (-not $holdProc.HasExited) { $holdProc.Kill() }
& $adb reboot 2>&1
Write-Host "Finished: $out"
