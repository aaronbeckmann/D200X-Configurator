# INSTALL: writes the deckd image to the `res` partition (mtd3) through the
# vendor's own flasher, the same path Ulanzi Studio uses for app updates.
# Only `res` is written. Bootloader, kernel, rootfs, UDISK (factory image for
# reset-key recovery) and data are untouched. ADB stays available at boot.
#
# Sequence (boot ADB window, stock app held dead):
#   push update.img to /tmp, setprop sys.zkupgrade.dir=/tmp, sys.zkupgrade.flag=255,
#   release the hold -> init restarts zkswe -> zkgui loader runs zk_upgrade_check,
#   flashes mtd3, verifies MD5 on read-back, reboots. Next boot loads our shim -> deckd.
#
# Usage: powershell -ExecutionPolicy Bypass -File research\adb-install.ps1 [-Image <update.img>]
#   default image: d200x-device\image\out\update.img (deckd)
#   to go back to stock: -Image "C:\Program Files (x86)\Ulanzi Studio\Ulanzi\UlanziDeck\defProfile\D200X\bin\02\01\update.img"
#   (extract bin.zip first) or any vendor update.img for device code 0xaa550606.
param([string]$Image = "", [string]$Serial = "")
$adb = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Google.PlatformTools*\platform-tools\adb.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
if (-not $adb) { $adb = "adb" }
$root = Split-Path $PSScriptRoot -Parent
$img = if ($Image) { $Image } else { Join-Path $root "d200x-device\image\out\update.img" }
$tool = Join-Path $root "d200x-tool\target\debug\d200x-tool.exe"
if (-not (Test-Path $img)) { Write-Host "build the image first (d200x-device\image\mkimage.py)"; exit 1 }
$outRoot = Join-Path $PSScriptRoot "adb-install"
New-Item -ItemType Directory -Force $outRoot | Out-Null
$ts = Get-Date -Format 'yyyyMMdd_HHmmss'
$out = Join-Path $outRoot $ts
New-Item -ItemType Directory -Force $out | Out-Null
& $adb kill-server 2>$null | Out-Null
# Detached, so the adb server daemon does not inherit this script's stdout pipe (which would
# keep the configurator's Firmware window waiting for EOF after the script has finished).
# (not -Wait: that would also wait for the daemon itself, which never exits)
$srv = Start-Process -FilePath $adb -ArgumentList "start-server" -WindowStyle Hidden -PassThru
$srv.WaitForExit(15000) | Out-Null

# Capture the serial from the running firmware (stock reports the real one) so
# deckd can present it; stored in /data/deckd/serial in the boot window.
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
if ($Serial) { & $adb shell "mkdir -p /data/deckd; echo -n $Serial > /data/deckd/serial" | Out-Null }
# Pre-flash snapshot of mtd3 (res) for the record.
& $adb shell "cp /dev/block/mtdblock3 /tmp/mtd3_before.bin" | Out-Null
& $adb pull /tmp/mtd3_before.bin (Join-Path $out "mtd3_before.bin") 2>&1 | Select-Object -Last 1
& $adb shell "rm /tmp/mtd3_before.bin" | Out-Null
& $adb push $img /tmp/update.img 2>&1 | Select-Object -Last 1
$remoteMd5 = (& $adb shell "busybox md5sum /tmp/update.img 2>/dev/null || echo nomd5")
$localMd5 = (Get-FileHash $img -Algorithm MD5).Hash.ToLower()
Write-Host "local md5 $localMd5 / remote $remoteMd5"
if ($remoteMd5 -notmatch "nomd5" -and $remoteMd5 -notmatch $localMd5) { Write-Host "push verification failed, aborting"; if (-not $holdProc.HasExited) { $holdProc.Kill() }; & $adb reboot; exit 1 }
& $adb shell "setprop sys.zkupgrade.dir /tmp; setprop sys.zkupgrade.flag 255; getprop sys.zkupgrade.dir; getprop sys.zkupgrade.flag" 2>&1 | Tee-Object -FilePath (Join-Path $out "props.txt")
Write-Host "Upgrade flagged. Releasing the hold so the vendor loader runs the flasher..."
if (-not $holdProc.HasExited) { $holdProc.Kill() }
Start-Sleep -Milliseconds 500
& $adb shell "setprop ctl.start zkswe" 2>&1 | Out-Null
# Watch for the reboot / reappearance.
Write-Host "Waiting up to 120 s for the deck to flash and reboot..."
$deadline = (Get-Date).AddSeconds(120)
$sawGone = $false
do {
  Start-Sleep -Seconds 2
  $adbp = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -like 'USB\VID_18D1*' }
  $hid = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -like 'USB\VID_2207&PID_0019\*' }
  if (-not $adbp -and -not $hid) { $sawGone = $true }
  $t = Get-Date -Format HH:mm:ss
  Write-Host "  $t adb=$($adbp -ne $null) hid=$($hid -ne $null)"
} until (($sawGone -and $hid) -or (Get-Date) -gt $deadline)
if ($hid) {
  Start-Sleep -Seconds 4
  Write-Host "--- info from the deck after reboot:"
  & $tool info 2>&1 | Tee-Object -FilePath (Join-Path $out "info_after.txt")
} else {
  Write-Host "Deck did not come back as HID within the timeout. Check the screen; ADB will be available on the next boot window."
}
Write-Host "Finished: $out"
