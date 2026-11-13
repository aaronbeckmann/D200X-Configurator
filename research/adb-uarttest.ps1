# UART test: find where ttyS0 (the SoC console) comes out on the deck board.
# Boot ADB window, deck app held dead, then a loop writes a test pattern to /dev/ttyS0
# (115200 8N1, the console's own setting) for -Seconds, then the deck reboots back into its
# installed firmware. Nothing is written to flash.
#
# Pattern per line: 64 x 'U' (0x55 = clean square wave, one bit = 8.68 us) + " ttyS0 test <n>" CRLF.
# Scope: the console TX pad shows continuous bursts. A USB-serial adapter at 115200 shows the text.
# Also records the UART driver's tx/rx counters before and after (proof the bytes left the SoC).
#
# Usage: powershell -ExecutionPolicy Bypass -File research\adb-uarttest.ps1 [-Seconds 180]
#   deck connected and running deckd/stock: it is rebooted by d200x-tool; otherwise replug it.
param([int]$Seconds = 180)
$adb = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Google.PlatformTools*\platform-tools\adb.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
if (-not $adb) { $adb = "adb" }
$root = Split-Path $PSScriptRoot -Parent
$tool = Join-Path $root "d200x-tool\target\debug\d200x-tool.exe"
$outRoot = Join-Path $PSScriptRoot "adb-uarttest"
$ts = Get-Date -Format 'yyyyMMdd_HHmmss'
$out = Join-Path $outRoot $ts
New-Item -ItemType Directory -Force $out | Out-Null
& $adb kill-server 2>$null | Out-Null
$srv = Start-Process -FilePath $adb -ArgumentList "start-server" -WindowStyle Hidden -PassThru
$srv.WaitForExit(15000) | Out-Null

$hold = "setprop sys.zkapp.state running; " +
        "while :; do setprop ctl.stop zkswe; " +
        "for p in `$(ps | busybox awk '/zkgui|deckd/ && !/awk/ {print `$1}'); do kill -9 `$p; done; done"
$holdProc = Start-Process -FilePath $adb -ArgumentList @("wait-for-device", "shell", "`"$hold`"") -NoNewWindow -PassThru -RedirectStandardOutput (Join-Path $out "hold.log")

$hid = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -like 'USB\VID_2207&PID_0019\*' }
if ($hid) {
  Write-Host "Deck connected: rebooting it into the ADB window..."
  & $tool reboot 2>&1 | Out-Null
} else {
  Write-Host "Deck not connected: plug it in now (waiting for ADB)..."
}
& $adb wait-for-device
Start-Sleep -Seconds 8
$state = (& $adb get-state 2>&1)
if ($state -ne "device") { Write-Host "ADB went away. Run again."; if (-not $holdProc.HasExited) { $holdProc.Kill() }; exit 1 }
Write-Host "ADB held."

& $adb shell "cat /proc/tty/driver/* 2>/dev/null; echo ---; busybox stty -F /dev/ttyS0 -a 2>&1; echo ---; ps | busybox grep -v ' \[' " > (Join-Path $out "before.txt") 2>&1

$u = "U" * 64
$loop = "(trap '' HUP; n=0; end=`$((`$(busybox date +%s) + $Seconds)); " +
        "while [ `$(busybox date +%s) -lt `$end ]; do n=`$((n+1)); " +
        "echo -e '$u ttyS0 test '`$n'\r' > /dev/ttyS0; done; " +
        "cat /proc/tty/driver/* > /tmp/uart_after.txt 2>&1; echo `$n > /tmp/uart_lines.txt) >/dev/null 2>&1 &"
& $adb shell $loop
$t0 = Get-Date
Write-Host ""
Write-Host "=== Writing 'UUUU... ttyS0 test <n>' to /dev/ttyS0 at 115200 for $Seconds s ==="
Write-Host "    Probe the TP3 holes now: the console TX shows continuous 8.7 us bit bursts."
Write-Host ""
while (((Get-Date) - $t0).TotalSeconds -lt ($Seconds + 3)) {
  $left = [int]($Seconds - ((Get-Date) - $t0).TotalSeconds)
  if ($left -ge 0 -and $left % 30 -eq 0) { Write-Host "  $left s left" }
  Start-Sleep -Seconds 1
}
& $adb shell "cat /tmp/uart_lines.txt; echo ---; cat /tmp/uart_after.txt" > (Join-Path $out "after.txt") 2>&1
Write-Host "--- driver counters before:"; Get-Content (Join-Path $out "before.txt") | Select-Object -First 6
Write-Host "--- lines written and counters after:"; Get-Content (Join-Path $out "after.txt")
if (-not $holdProc.HasExited) { $holdProc.Kill() }
& $adb reboot 2>&1 | Out-Null
Write-Host "Deck rebooting into its installed firmware. Results: $out"
