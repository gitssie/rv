$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '../..')
$installers = @(Get-ChildItem dist/*-setup.exe)
if ($installers.Count -ne 1) { throw 'Expected exactly one installer' }
# A custom path with spaces exercises the user-selected installation directory.
$installation = Join-Path $env:RUNNER_TEMP "RV install test $([guid]::NewGuid().ToString('N'))"
$arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', "/DIR=`"$installation`"")
function Install-RV {
    $process = Start-Process -FilePath $installers[0].FullName -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installer failed: $($process.ExitCode)" }
    if (-not (Test-Path (Join-Path $installation 'rv.exe'))) { throw 'Installed rv.exe is missing' }
}
Install-RV
$data = Join-Path $installation 'data'
if (-not (Test-Path $data -PathType Container)) { throw 'Data directory must be beside rv.exe' }
$sentinel = Join-Path $data 'keep-on-upgrade-and-uninstall.txt'
Set-Content $sentinel 'RV data preservation test' -Encoding utf8
$hash = (Get-FileHash $sentinel).Hash
Install-RV
if ((Get-FileHash $sentinel).Hash -ne $hash) { throw 'Upgrade changed user data' }
$uninstaller = Join-Path $installation 'unins000.exe'
$process = Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "Uninstall failed: $($process.ExitCode)" }
# Inno's uninstaller may exit before its temporary child finishes cleaning up.
for ($attempt = 0; $attempt -lt 20 -and (Test-Path (Join-Path $installation 'rv.exe')); $attempt++) { Start-Sleep -Milliseconds 250 }
if (Test-Path (Join-Path $installation 'rv.exe')) { throw 'Uninstall did not remove program files' }
if ((Get-FileHash $sentinel).Hash -ne $hash) { throw 'Uninstall removed or changed user data' }
Write-Host 'Install, same-version upgrade, custom directory and data-preserving uninstall passed.'
