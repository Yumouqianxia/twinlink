param(
  [string]$Tag = 'v0.0.1',
  [ValidateSet('normal', 'blocked-downloads', 'corrupt-database')][string]$Scenario = 'normal',
  [string]$InstallerPath,
  [switch]$RequireReady
)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or -not $env:RUNNER_TEMP) { throw 'This fixture is only for disposable GitHub Actions runners' }
if ($Tag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+$') { throw 'Invalid release tag' }
$resultDir = Join-Path $PWD "startup-results/$Scenario"
New-Item -ItemType Directory -Force $resultDir | Out-Null
if (-not $InstallerPath) {
  $downloadDir = Join-Path $env:RUNNER_TEMP 'twinlink-installer'
  New-Item -ItemType Directory -Force $downloadDir | Out-Null
  gh release download $Tag --repo Yumouqianxia/twinlink --pattern '*windows-x86_64-setup.exe' --dir $downloadDir
  if ($LASTEXITCODE -ne 0) { throw 'Installer download failed' }
  $installers = @(Get-ChildItem $downloadDir -Filter '*-setup.exe')
  if ($installers.Count -ne 1) { throw 'Expected exactly one installer' }
  $InstallerPath = $installers[0].FullName
}
$InstallerPath = (Resolve-Path $InstallerPath).Path
$installDir = Join-Path $env:RUNNER_TEMP 'TwinLinkInstalledSmoke'
$install = Start-Process -FilePath $InstallerPath -ArgumentList @('/S', "/D=$installDir") -PassThru
if (-not $install.WaitForExit(300000)) { Stop-Process -Id $install.Id -Force; throw 'Installer timed out' }
if ($install.ExitCode -ne 0) { throw "Installer exited with $($install.ExitCode)" }
$exe = Join-Path $installDir 'twinlink.exe'
if (-not (Test-Path $exe)) { throw 'Installed twinlink.exe missing' }
Get-ChildItem -Recurse $installDir | Select-Object FullName,Length | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $resultDir 'installed-files.json')
$stdout = Join-Path $resultDir 'stdout.txt'
$stderr = Join-Path $resultDir 'stderr.txt'
$env:TWINLINK_NO_ERROR_DIALOG = '1'
$log = Join-Path $env:LOCALAPPDATA 'TwinLink\logs\startup.log'
if (Test-Path $log) { Remove-Item $log }
$folderKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders'
$folderName = '{374DE290-123F-4565-9164-39C4925E467B}'
$oldDownload = (Get-Item $folderKey).GetValue($folderName, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
$oldKind = if ($null -ne $oldDownload) { (Get-Item $folderKey).GetValueKind($folderName).ToString() } else { $null }
$app = $null
try {
  if ($Scenario -eq 'blocked-downloads') {
    $blocked = Join-Path $env:RUNNER_TEMP 'not-a-download-directory'
    Set-Content -Path $blocked -Value 'existing test file must not be overwritten'
    New-ItemProperty -Path $folderKey -Name $folderName -Value $blocked -PropertyType ExpandString -Force | Out-Null
  }
  $database = Join-Path $env:APPDATA 'app.twinlink.desktop\twinlink.db'
  if ($Scenario -eq 'corrupt-database') {
    New-Item -ItemType Directory -Force (Split-Path $database) | Out-Null
    foreach ($suffix in @('', '-wal', '-shm')) { Remove-Item "$database$suffix" -ErrorAction SilentlyContinue }
    [IO.File]::WriteAllText($database, 'TwinLink test: not a SQLite database')
  }
  $app = Start-Process -FilePath $exe -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
  $earlyExit = $app.WaitForExit(20000)
  $content = if (Test-Path $log) { Get-Content $log -Raw } else { '' }
  $report = @{
    tag = $Tag
    scenario = $Scenario
    os = [Environment]::OSVersion.VersionString
    processSurvived20Seconds = -not $earlyExit
    frontendReady = $content.Contains('[frontend-ready]')
    downloadFallbackUsed = $content.Contains('[downloads-fallback]')
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
  $report | ConvertTo-Json -Depth 4 | Tee-Object -FilePath (Join-Path $resultDir 'result.json')
  Get-Content $stderr
  if (Test-Path $log) { Copy-Item $log (Join-Path $resultDir 'startup.log') }
  if ($Scenario -eq 'corrupt-database' -and $RequireReady) {
    if (-not $earlyExit -or $app.ExitCode -ne 1 -or -not $content.Contains('[setup-error]')) { throw 'Expected a controlled startup failure with diagnostics' }
    if ($content.Contains('[panic]')) { throw 'Setup failure must not become an unhandled panic' }
    if ([IO.File]::ReadAllText($database) -ne 'TwinLink test: not a SQLite database') { throw 'Existing database was modified or replaced' }
    return
  }
  if ($earlyExit) { throw "Installed application exited early: $($app.ExitCode)" }
  if ($RequireReady -and -not $report.frontendReady) { throw 'Frontend did not reach real Tauri snapshot IPC' }
  if ($RequireReady -and $Scenario -eq 'blocked-downloads' -and -not $report.downloadFallbackUsed) { throw 'Download-directory fallback was not exercised' }
} finally {
  if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force; $app.WaitForExit(5000) | Out-Null }
  if ($Scenario -eq 'blocked-downloads') {
    if ($null -eq $oldDownload) { Remove-ItemProperty -Path $folderKey -Name $folderName -ErrorAction SilentlyContinue }
    else { New-ItemProperty -Path $folderKey -Name $folderName -Value $oldDownload -PropertyType $oldKind -Force | Out-Null }
  }
}
