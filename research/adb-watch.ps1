# Watches for the deck's transient ADB interface (18d1:d002) and, when it
# appears, runs a read-only information dump into research/adb/<timestamp>/.
# Usage: powershell -ExecutionPolicy Bypass -File research\adb-watch.ps1 [-Once]
param([switch]$Once, [int]$PollMs = 250)

$adb = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Google.PlatformTools*\platform-tools\adb.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
if (-not $adb) { $adb = "adb" }
$outRoot = Join-Path $PSScriptRoot "adb"
New-Item -ItemType Directory -Force $outRoot | Out-Null
& $adb start-server 2>$null | Out-Null

$readonlyCmds = @(
  "id; uname -a; cat /proc/version; cat /proc/cmdline",
  "cat /proc/cpuinfo; cat /proc/meminfo | head -5",
  "cat /proc/mtd; cat /proc/partitions; cat /proc/mounts",
  "ls -la / /dev /dev/input /dev/block 2>/dev/null",
  "ls -la /sys/class/graphics 2>/dev/null; cat /sys/class/graphics/fb0/virtual_size /sys/class/graphics/fb0/bits_per_pixel /sys/class/graphics/fb0/stride 2>/dev/null",
  "cat /sys/devices/soc0/soc/soc:usbotg/otg_role 2>/dev/null; cat /sys/class/zkswe_usb/zkswe0/iSerial 2>/dev/null",
  "ls -la /res /res/etc /res/lib /customer /config /data /appconfigs /mnt/storage 2>/dev/null",
  "cat /res/etc/EasyUI.cfg 2>/dev/null; cat /etc/fw_env.config 2>/dev/null",
  "ps; getprop 2>/dev/null; cat /etc/init.d/* 2>/dev/null | head -200",
  "dmesg"
)

Write-Host "Watching for 18d1:d002 (deck ADB). Unplug/replug the deck to trigger a boot. Ctrl-C to stop."
while ($true) {
  $dev = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -like 'USB\VID_18D1&PID_D002*' }
  if ($dev) {
    $ts = Get-Date -Format 'yyyyMMdd_HHmmss'
    $out = Join-Path $outRoot $ts
    New-Item -ItemType Directory -Force $out | Out-Null
    Write-Host "[$ts] ADB device present: $($dev.InstanceId) -> dumping to $out"
    & $adb devices | Out-File (Join-Path $out "devices.txt")
    $i = 0
    foreach ($c in $readonlyCmds) {
      $i++
      $f = Join-Path $out ("{0:d2}.txt" -f $i)
      "`$ $c" | Out-File $f
      & $adb shell $c 2>&1 | Out-File -Append $f
      if ($LASTEXITCODE -ne 0) { "[adb exit $LASTEXITCODE]" | Out-File -Append $f }
    }
    Write-Host "[$ts] dump finished"
    if ($Once) { break }
    Start-Sleep -Seconds 10
  }
  Start-Sleep -Milliseconds $PollMs
}
