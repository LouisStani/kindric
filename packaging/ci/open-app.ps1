# Opens kindric-desktop on the screen of this Windows machine, as nobody did by hand: waits for its
# page to be ready (written in trace.log), then takes a screenshot, whether it came or not.
#
# usage: packaging/ci/open-app.ps1 <dir with the programs> <dir for the screenshot and the app's output>
param([string]$Bin, [string]$Out)
$ErrorActionPreference = 'Stop'

$Bin = (Resolve-Path $Bin).Path
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path
# The data folder of kindric on Windows: the roaming AppData, as `dirs::data_dir` gives it.
$trace = Join-Path $env:APPDATA 'kindric\trace.log'
Remove-Item $trace -ErrorAction SilentlyContinue

$env:KINDRIC_TRACE = '1'
$env:PATH = "$Bin;$env:PATH"
$app = Start-Process -FilePath (Join-Path $Bin 'kindric-desktop.exe') -PassThru `
    -RedirectStandardOutput (Join-Path $Out 'app.out') -RedirectStandardError (Join-Path $Out 'app.err')
$ready = $false
for ($i = 0; $i -lt 120; $i++) {
    if ((Test-Path $trace) -and (Select-String -Path $trace -Pattern 'page ready' -Quiet)) {
        $ready = $true
        break
    }
    if ($app.HasExited) { break }
    Start-Sleep -Milliseconds 500
}
# The page draws a moment after it says it is ready.
Start-Sleep -Seconds 3
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
$bitmap.Save((Join-Path $Out 'windows.png'), [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose()
$bitmap.Dispose()
if (-not $app.HasExited) { Stop-Process -Id $app.Id -Force }

if ($ready) {
    Write-Host 'the page is ready:'
    Select-String -Path $trace -Pattern 'page ready' | ForEach-Object { Write-Host $_.Line }
} else {
    Get-Content (Join-Path $Out 'app.err') -ErrorAction SilentlyContinue | Write-Host
    throw "no `"page ready`" in $trace after 60 s"
}
