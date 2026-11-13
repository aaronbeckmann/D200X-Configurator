# After the deck is running the host-action build of deckd: upload the rendered
# icons, push the Home Assistant test config, and start the agent (background).
$root = Split-Path $PSScriptRoot -Parent
$tool = Join-Path $root "d200x-tool\target\debug\d200x-tool.exe"
$agent = Join-Path $root "d200x-agent\target\release\d200x-agent.exe"
Get-ChildItem (Join-Path $PSScriptRoot "icons") -Filter *.png | ForEach-Object {
  Write-Host ("icon {0}: " -f $_.Name) -NoNewline
  & $tool set-icon $_.Name $_.FullName
}
Write-Host "config: " -NoNewline
& $tool set-config (Join-Path $PSScriptRoot "deckd-ha-config.json")
Get-Process d200x-agent -ErrorAction SilentlyContinue | Stop-Process -Force
$p = Start-Process -FilePath $agent -WindowStyle Hidden -PassThru
Write-Host "agent started, pid $($p.Id); log: $env:APPDATA\d200x\agent.log"
Start-Sleep -Seconds 6
Get-Content "$env:APPDATA\d200x\agent.log" -Tail 12
