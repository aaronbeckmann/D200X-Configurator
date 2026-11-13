# Test run of deckd (d200x-device/deckd) from /tmp with the stock app held dead.
# 1. boot window: fetch previous deckd log, push binary, launch detached with --seconds N
# 2. deckd switches the gadget to HID (ADB goes away, expected)
# 3. from the PC: d200x-tool info / get-config / set-config / get-config
# 4. you press keys on the deck (Notepad focused) until the deck reboots itself.
#
# Usage: powershell -ExecutionPolicy Bypass -File research\adb-deckd.ps1 [-Seconds 240]
param([int]$Seconds = 240)

$adb = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Google.PlatformTools*\platform-tools\adb.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
if (-not $adb) { $adb = "adb" }
$root = Split-Path $PSScriptRoot -Parent
$bin = Join-Path $root "d200x-device\deckd\target\armv7-unknown-linux-musleabihf\release\deckd"
$tool = Join-Path $root "d200x-tool\target\debug\d200x-tool.exe"
$testcfg = Join-Path $PSScriptRoot "deckd-test-config.json"
if (-not (Test-Path $bin)) { Write-Host "build deckd first"; exit 1 }
$outRoot = Join-Path $PSScriptRoot "adb-deckd"
New-Item -ItemType Directory -Force $outRoot | Out-Null
$ts = Get-Date -Format 'yyyyMMdd_HHmmss'
$out = Join-Path $outRoot $ts
New-Item -ItemType Directory -Force $out | Out-Null
& $adb kill-server 2>$null | Out-Null
& $adb start-server 2>$null | Out-Null

$hold = "setprop sys.zkapp.state running; " +
        "while :; do setprop ctl.stop zkswe; " +
        "for p in `$(ps | busybox awk '/zkgui/ && !/awk/ {print `$1}'); do kill -9 `$p; done; done"
Write-Host "Waiting for the deck's ADB interface (unplug/replug the deck now)..."
$holdProc = Start-Process -FilePath $adb -ArgumentList @("wait-for-device", "shell", "`"$hold`"") -NoNewWindow -PassThru -RedirectStandardOutput (Join-Path $out "hold.log")
& $adb wait-for-device
Start-Sleep -Seconds 8
$state = (& $adb get-state 2>&1)
if ($state -ne "device") { Write-Host "ADB went away. Replug and try again."; if (-not $holdProc.HasExited) { $holdProc.Kill() }; exit 1 }

& $adb pull /data/deckd/deckd.log (Join-Path $out "deckd_prev.log") 2>&1 | Select-Object -Last 1
& $adb pull /data/deckd/config.json (Join-Path $out "config_prev.json") 2>&1 | Select-Object -Last 1
$fontOnDeck = (& $adb shell "ls /data/deckd/font.ttf 2>/dev/null")
if (-not $fontOnDeck) {
  & $adb shell "mkdir -p /data/deckd" | Out-Null
  & $adb push C:\Windows\Fonts\consola.ttf /data/deckd/font.ttf 2>&1 | Select-Object -Last 1
}
& $adb push $bin /tmp/deckd 2>&1 | Select-Object -Last 1
& $adb shell "chmod 755 /tmp/deckd; (trap '' HUP; /tmp/deckd --seconds $Seconds) >/dev/null 2>&1 &"
Start-Sleep -Seconds 1
if (-not $holdProc.HasExited) { $holdProc.Kill() }
Write-Host "deckd launched for $Seconds s. Waiting for the HID gadget..."

$deadline = (Get-Date).AddSeconds(40)
do {
  Start-Sleep -Seconds 1
  $hid = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -like 'USB\VID_2207&PID_0019\*' }
} until ($hid -or (Get-Date) -gt $deadline)
if (-not $hid) { Write-Host "HID gadget did not appear."; exit 1 }
Start-Sleep -Seconds 3
Write-Host "--- info"
& $tool info 2>&1 | Tee-Object -FilePath (Join-Path $out "01_info.txt")
Write-Host "--- get-config (default)"
& $tool get-config (Join-Path $out "02_config_default.json") 2>&1 | Tee-Object -FilePath (Join-Path $out "02_get.txt")
Write-Host "--- set-config (test config)"
& $tool set-config $testcfg 2>&1 | Tee-Object -FilePath (Join-Path $out "03_set.txt")
Write-Host "--- get-config (after push)"
& $tool get-config (Join-Path $out "04_config_after.json") 2>&1 | Tee-Object -FilePath (Join-Path $out "04_get.txt")
Write-Host "--- brightness 60 via stock command"
& $tool brightness 60 2>&1 | Tee-Object -FilePath (Join-Path $out "05_brightness.txt")
Write-Host "Done with host side. Press keys on the deck now (Notepad focused). Deck reboots itself at the end of the run."
Write-Host "Finished: $out"
