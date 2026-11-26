# Runs packaging/install.ps1 against an archive served on localhost: a damaged download, an install,
# an update, the uninstall.
#
# usage: packaging/ci/test-install.ps1 <dir with kindric-x86_64-pc-windows-msvc.zip and SHA256SUMS>
param([Parameter(Mandatory)][string]$Dist)

$ErrorActionPreference = 'Stop'
$Dist = (Resolve-Path $Dist).Path
$Installer = Join-Path $PSScriptRoot '..\install.ps1'
$Port = 8765
$script:Failures = 0

function Check([bool]$Ok, [string]$What) {
    if ($Ok) { Write-Host "ok   $What" } else { Write-Host "FAIL $What"; $script:Failures++ }
}

$server = Start-Process python -ArgumentList '-m', 'http.server', $Port, '--bind', '127.0.0.1', '--directory', $Dist -PassThru -WindowStyle Hidden
try {
    $ready = $false
    foreach ($try in 1..50) {
        try { Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:$Port/SHA256SUMS" | Out-Null; $ready = $true; break } catch { Start-Sleep -Milliseconds 100 }
    }
    if (-not $ready) { throw "the test server did not start" }

    $bin = Join-Path ([IO.Path]::GetTempPath()) ("kindric-bin-" + [Guid]::NewGuid())
    $env:KINDRIC_INSTALL_DIR = $bin
    $env:KINDRIC_DOWNLOAD_URL = "http://127.0.0.1:$Port"
    $shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'kindric.lnk'

    # A damaged download is refused, and nothing is installed.
    $sums = Join-Path $Dist 'SHA256SUMS'
    $good = Get-Content $sums
    $good | ForEach-Object { ('0' * 64) + $_.Substring(64) } | Set-Content $sums
    $failed = $false
    try { & $Installer } catch { $failed = $true; $message = "$_" }
    $good | Set-Content $sums
    Check $failed 'a damaged download fails'
    Check ($message -match 'does not match its SHA256SUMS') 'and says why'
    Check (-not (Test-Path (Join-Path $bin 'kindric.exe'))) 'and installs nothing'

    & $Installer
    foreach ($name in 'kindric.exe', 'kindric-accounts.exe', 'kindric-desktop.exe') {
        Check (Test-Path (Join-Path $bin $name)) "$name is installed"
    }
    Check ((& (Join-Path $bin 'kindric.exe') --version) -match '^kindric ') 'kindric runs'
    Check ((& (Join-Path $bin 'kindric-accounts.exe') --version) -match '^kindric-accounts ') 'kindric-accounts runs'
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User') -split ';'
    Check ($userPath -contains $bin) 'its folder is on the user PATH'
    Check (Test-Path $shortcut) 'the Start menu entry is made'
    $link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
    Check ($link.TargetPath -eq (Join-Path $bin 'kindric-desktop.exe')) 'it starts kindric-desktop'

    # Run as the README runs it, through iex: it updates, and leaves the session as it was.
    $left = & {
        $ErrorActionPreference = 'Continue'
        Invoke-Expression (Get-Content -Raw $Installer)
        $ErrorActionPreference
        [bool](Get-Command Install-Kindric -ErrorAction SilentlyContinue)
    }
    Check ($left[-2] -eq 'Continue') 'through iex, it leaves the preferences of the session alone'
    Check (-not $left[-1]) 'and defines no function in it'
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User') -split ';'
    Check (@($userPath | Where-Object { $_ -eq $bin }).Count -eq 1) 'a second run updates without adding the folder twice'

    & $Installer -Uninstall
    Check (-not (Test-Path (Join-Path $bin 'kindric.exe'))) 'uninstall removes the programs'
    Check (-not (Get-ChildItem $bin -Filter '*.old-*' -ErrorAction SilentlyContinue)) 'and the old ones moved aside'
    Check (-not (Test-Path $shortcut)) 'and the Start menu entry'
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User') -split ';'
    Check (-not ($userPath -contains $bin)) 'and the PATH entry'
} finally {
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
}

if ($script:Failures -gt 0) {
    Write-Host "$($script:Failures) check(s) failed"
    exit 1
}
Write-Host 'all checks passed'
