# Smoke test of a built binary on Windows. Uses the runner's own profile (the CI
# machine is thrown away afterwards) and a fake claude.cmd. No real account, no network.
# Usage: ci/smoke.ps1 <path to claude-accounts.exe>
param([Parameter(Mandatory = $true)][string]$Bin)
$ErrorActionPreference = 'Stop'
$Bin = (Resolve-Path $Bin).Path

function Fail([string]$Message) {
    Write-Host "FAIL: $Message"
    exit 1
}

$shared = Join-Path $env:USERPROFILE '.claude'
New-Item -ItemType Directory -Force (Join-Path $shared 'skills') | Out-Null
Set-Content -Path (Join-Path $shared 'CLAUDE.md') -Value '# shared'
$fake = Join-Path $env:RUNNER_TEMP 'fake-claude'
New-Item -ItemType Directory -Force $fake | Out-Null
Set-Content -Path (Join-Path $fake 'claude.cmd') -Value "@echo CONFIG=%CLAUDE_CONFIG_DIR%`r`n@echo APIKEY=[%ANTHROPIC_API_KEY%]`r`n@echo ARGS=%*"
$env:PATH = "$fake;$env:PATH"

& $Bin add ci1 | Out-Null
if ($LASTEXITCODE -ne 0) { Fail 'add' }
$list = (& $Bin list --json | Out-String).Trim()
if ($list -ne '["ci1"]') { Fail "list --json: $list" }
$dir = Join-Path $env:APPDATA 'claude-accounts\ci1'
if (-not (Test-Path (Join-Path $dir 'skills'))) { Fail 'skills is not shared' }
if ((Get-Content (Join-Path $dir 'CLAUDE.md') -Raw).Trim() -ne '# shared') { Fail 'CLAUDE.md is not shared' }

$env:ANTHROPIC_API_KEY = 'secret'
$out = & $Bin run ci1 --resume | Out-String
Remove-Item Env:ANTHROPIC_API_KEY
if ($out -notmatch [regex]::Escape("CONFIG=$dir")) { Fail "wrong CLAUDE_CONFIG_DIR: $out" }
if ($out -match 'secret') { Fail "ANTHROPIC_API_KEY not removed: $out" }
if ($out -notmatch 'ARGS=--resume') { Fail "arguments not passed: $out" }

& $Bin isolate ci1 | Out-Null
$isolated = (& $Bin isolated --json | Out-String).Trim()
if ($isolated -ne '["ci1"]') { Fail "isolated --json: $isolated" }
Write-Host 'smoke test OK'
