# Installs kindric on Windows: kindric-desktop, kindric and kindric-accounts, from a release of
# github.com/LouisStani/kindric.
#
#   irm https://github.com/LouisStani/kindric/releases/latest/download/install.ps1 | iex
#
# With options:
#   & ([scriptblock]::Create((irm https://github.com/LouisStani/kindric/releases/latest/download/install.ps1))) -Version 1.0.0
#   & ([scriptblock]::Create((irm https://github.com/LouisStani/kindric/releases/latest/download/install.ps1))) -Uninstall
#
# KINDRIC_INSTALL_DIR changes where the programs go (default %LOCALAPPDATA%\Programs\kindric).
# KINDRIC_DOWNLOAD_URL replaces the folder of the release (used by the tests).
param(
    [string]$Version = "",
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Repo = 'LouisStani/kindric'
$Target = 'x86_64-pc-windows-msvc'
$Programs = @('kindric.exe', 'kindric-accounts.exe', 'kindric-desktop.exe')
$WebView2 = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'

$BinDir = if ($env:KINDRIC_INSTALL_DIR) { $env:KINDRIC_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\kindric' }
$Shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'kindric.lnk'

# throw, not exit: run through iex, exit would close the user's PowerShell window.
function Fail([string]$Message) {
    throw "kindric install: $Message"
}

function Get-UserPath {
    $value = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $value) { return @() }
    return @($value -split ';' | Where-Object { $_ -ne '' })
}

function Test-SameDir([string]$A, [string]$B) {
    return $A.TrimEnd('\') -ieq $B.TrimEnd('\')
}

function Test-OurShortcut {
    if (-not (Test-Path $Shortcut)) { return $false }
    $link = (New-Object -ComObject WScript.Shell).CreateShortcut($Shortcut)
    if (-not $link.TargetPath) { return $false }
    return Test-SameDir (Split-Path $link.TargetPath) $BinDir
}

function Install-Kindric {
    if ($env:KINDRIC_DOWNLOAD_URL) {
        $base = $env:KINDRIC_DOWNLOAD_URL
    } elseif ($Version) {
        $base = "https://github.com/$Repo/releases/download/v$($Version.TrimStart('v'))"
    } else {
        $base = "https://github.com/$Repo/releases/latest/download"
    }
    # Windows PowerShell 5.1 may still offer only TLS 1.0, which GitHub refuses.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $archive = "kindric-$Target.zip"
    $work = Join-Path ([IO.Path]::GetTempPath()) ("kindric-" + [Guid]::NewGuid())
    New-Item -ItemType Directory -Force $work | Out-Null
    try {
        Write-Host "Downloading kindric for $Target..."
        try {
            Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile (Join-Path $work 'SHA256SUMS')
            Invoke-WebRequest -UseBasicParsing -Uri "$base/$archive" -OutFile (Join-Path $work $archive)
        } catch {
            Fail "could not download from ${base}: $($_.Exception.Message)"
        }
        $line = Get-Content (Join-Path $work 'SHA256SUMS') | Where-Object { $_ -match " $([regex]::Escape($archive))$" } | Select-Object -First 1
        if (-not $line) { Fail "$archive is not listed in the SHA256SUMS of the release" }
        $expected = ($line -split ' ')[0].ToLower()
        $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $work $archive)).Hash.ToLower()
        if ($actual -ne $expected) { Fail "$archive does not match its SHA256SUMS: the download is damaged, try again" }

        Expand-Archive -Path (Join-Path $work $archive) -DestinationPath $work -Force
        $unpacked = Join-Path $work "kindric-$Target"
        New-Item -ItemType Directory -Force $BinDir | Out-Null
        foreach ($name in $Programs) {
            try {
                Copy-Item -Force (Join-Path $unpacked $name) (Join-Path $BinDir $name)
            } catch {
                Fail "could not replace $name in ${BinDir}: close kindric, then run this again ($($_.Exception.Message))"
            }
        }
    } finally {
        Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
    }

    $userPath = Get-UserPath
    if (-not ($userPath | Where-Object { Test-SameDir $_ $BinDir })) {
        [Environment]::SetEnvironmentVariable('Path', (($userPath + $BinDir) -join ';'), 'User')
    }
    if (-not (($env:Path -split ';') | Where-Object { $_ -and (Test-SameDir $_ $BinDir) })) {
        $env:Path = "$env:Path;$BinDir"
    }

    if ((Test-Path $Shortcut) -and -not (Test-OurShortcut)) {
        Write-Host "warning: $Shortcut was not made by this script: left as it is, kindric gets no Start menu entry"
    } else {
        $link = (New-Object -ComObject WScript.Shell).CreateShortcut($Shortcut)
        $link.TargetPath = Join-Path $BinDir 'kindric-desktop.exe'
        $link.WorkingDirectory = $env:USERPROFILE
        $link.Description = 'Your Claude Code sessions, every account, one window'
        $link.Save()
    }

    $installed = & (Join-Path $BinDir 'kindric.exe') --version
    Write-Host "$installed installed in $BinDir."
    Write-Host "Open kindric from the Start menu, or run kindric-desktop in a new terminal."

    $hasWebView = $false
    foreach ($key in @("HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$WebView2", "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$WebView2", "HKCU:\Software\Microsoft\EdgeUpdate\Clients\$WebView2")) {
        $pv = (Get-ItemProperty -Path $key -Name pv -ErrorAction SilentlyContinue).pv
        if ($pv -and $pv -ne '0.0.0.0') { $hasWebView = $true }
    }
    if (-not $hasWebView) {
        Write-Host ""
        Write-Host "The desktop app needs Microsoft Edge WebView2, which was not found."
        Write-Host "Install the Evergreen runtime from https://developer.microsoft.com/microsoft-edge/webview2/"
    }
    if (-not (Get-Command claude -ErrorAction SilentlyContinue)) {
        Write-Host ""
        Write-Host "Claude Code is not installed: kindric shows its sessions, so install it too."
        Write-Host "See https://docs.anthropic.com/en/docs/claude-code"
    }
    foreach ($name in $Programs) {
        $found = Get-Command $name -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($found -and -not (Test-SameDir (Split-Path $found.Source) $BinDir)) {
            Write-Host ""
            Write-Host "warning: $($found.Source) comes before $BinDir in your PATH and will run instead."
            Write-Host "Remove the old one (cargo uninstall $($name -replace '\.exe$', ''), if it came from cargo)."
        }
    }
    Write-Host ""
    Write-Host "Open a new terminal for the PATH to apply. On first launch, kindric lists what is left to set up."
    Write-Host "Run this script again to update."
}

function Uninstall-Kindric {
    $removed = @()
    foreach ($name in $Programs) {
        $file = Join-Path $BinDir $name
        if (Test-Path $file) {
            try {
                Remove-Item -Force $file
            } catch {
                Fail "could not remove ${file}: close kindric, then run this again"
            }
            $removed += $name
        }
    }
    if (Test-OurShortcut) {
        Remove-Item -Force $Shortcut
        $removed += 'Start menu entry'
    }
    $userPath = Get-UserPath
    if ($userPath | Where-Object { Test-SameDir $_ $BinDir }) {
        [Environment]::SetEnvironmentVariable('Path', (($userPath | Where-Object { -not (Test-SameDir $_ $BinDir) }) -join ';'), 'User')
        $removed += 'PATH entry'
    }
    if ((Test-Path $BinDir) -and -not (Get-ChildItem $BinDir -Force)) {
        Remove-Item -Force $BinDir
    }
    if ($removed.Count -eq 0) {
        Write-Host "Nothing of kindric was found in $BinDir."
    } else {
        Write-Host "Removed: $($removed -join ', ')."
    }
    Write-Host "Your accounts, sessions, messages and settings stay, in kindric's and kindric-accounts' data folders"
    Write-Host "(see the installation guide to remove them too). Claude Code is not touched."
}

if ($Uninstall -or $env:KINDRIC_UNINSTALL) { Uninstall-Kindric } else { Install-Kindric }
