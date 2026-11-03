param([string]$Tag = 'v0.0.1')
$ErrorActionPreference = 'Stop'
if ($Tag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+$') { throw 'Invalid release tag' }
$resultDir = Join-Path $PWD 'startup-results'
New-Item -ItemType Directory -Force $resultDir | Out-Null
$downloadDir = Join-Path $env:RUNNER_TEMP 'twinlink-installer'
New-Item -ItemType Directory -Force $downloadDir | Out-Null
gh release download $Tag --repo Yumouqianxia/twinlink --pattern '*windows-x86_64-setup.exe' --dir $downloadDir
if ($LASTEXITCODE -ne 0) { throw 'Installer download failed' }
$installer = @(Get-ChildItem $downloadDir -Filter '*-setup.exe')
if ($installer.Count -ne 1) { throw 'Expected exactly one installer' }
$installDir = Join-Path $env:RUNNER_TEMP 'TwinLinkInstalledSmoke'
$install = Start-Process -FilePath $installer[0].FullName -ArgumentList @('/S', "/D=$installDir") -PassThru
if (-not $install.WaitForExit(300000)) { Stop-Process -Id $install.Id -Force; throw 'Installer timed out' }
if ($install.ExitCode -ne 0) { throw "Installer exited with $($install.ExitCode)" }
$exe = Join-Path $installDir 'twinlink.exe'
if (-not (Test-Path $exe)) { throw 'Installed twinlink.exe missing' }
Get-ChildItem -Recurse $installDir | Select-Object FullName,Length | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $resultDir 'installed-files.json')
$stdout = Join-Path $resultDir 'stdout.txt'
$stderr = Join-Path $resultDir 'stderr.txt'
$app = Start-Process -FilePath $exe -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
$earlyExit = $app.WaitForExit(20000)
$report = @{
  tag = $Tag
  os = [Environment]::OSVersion.VersionString
  processSurvived20Seconds = -not $earlyExit
  exitCode = $(if ($earlyExit) { $app.ExitCode } else { $null })
  runtimeRegistry = @()
}
foreach ($root in @('HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients', 'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients')) {
  if (Test-Path $root) {
    foreach ($entry in Get-ChildItem $root) {
      $value = Get-ItemProperty $entry.PSPath
      if ($value.name -like '*WebView*') { $report.runtimeRegistry += @{name=$value.name;version=$value.pv} }
    }
  }
}
if (-not $earlyExit) { Stop-Process -Id $app.Id -Force }
$report | ConvertTo-Json -Depth 4 | Tee-Object -FilePath (Join-Path $resultDir 'result.json')
Get-Content $stderr
if ($earlyExit) { throw "Installed application exited early: $($app.ExitCode)" }
