param([ValidateSet('x86_64', 'aarch64')][string]$Architecture = 'x86_64')
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
Set-Location (Join-Path $PSScriptRoot '../..')

$manifest = Get-Content Cargo.toml -Raw
$match = [regex]::Match($manifest, '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"')
if (-not $match.Success) { throw 'Workspace version is missing' }
$version = $match.Groups[1].Value
$label = $version
if ($env:GITHUB_REF_TYPE -eq 'tag') {
    $label = $env:GITHUB_REF_NAME -replace '^v', ''
} elseif ($env:GITHUB_SHA) {
    $label = "$version-$($env:GITHUB_SHA.Substring(0, 7))"
}
if ($label -notmatch '^[0-9A-Za-z][0-9A-Za-z._-]*$') { throw 'Invalid artifact version' }
$stem = "rv-$label-windows-$Architecture"
$stage = Join-Path $PWD "dist/$stem"
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item -Force -Path 'target/release/rv.exe', 'LICENSE', 'README.md' -Destination $stage

# Ship app-local MSVC runtime DLLs so the user needs no separate runtime download.
$runtimeArch = if ($Architecture -eq 'aarch64') { 'arm64' } else { 'x64' }
if (-not $env:VCToolsRedistDir) { throw 'MSVC redistributable directory is not configured' }
$runtime = Get-ChildItem (Join-Path $env:VCToolsRedistDir $runtimeArch) -Directory |
    Where-Object { $_.Name -match '^Microsoft\.VC[0-9]+\.CRT$' } | Select-Object -First 1
if (-not $runtime) { throw "MSVC $runtimeArch runtime DLLs were not found" }
Copy-Item -Force -Path (Join-Path $runtime.FullName '*.dll') -Destination $stage

$icon = Join-Path $PWD 'dist/app-icon.ico'
python -c "from PIL import Image; Image.open('assets/app-icon.png').save('dist/app-icon.ico', sizes=[(16,16),(32,32),(48,48),(64,64),(128,128),(256,256)])"
if ($LASTEXITCODE -ne 0) { throw 'Icon conversion failed' }
Copy-Item -Force $icon (Join-Path $stage 'app-icon.ico')
$compiler = 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe'
& $compiler "/DAppVersion=$version" "/DArtifactName=$stem-setup" "/DArchitecture=$Architecture" "/DStageDir=$stage" "/DAppIcon=$icon" scripts/windows/installer.iss
if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'data') | Out-Null
Compress-Archive -Force -Path $stage -DestinationPath "dist/$stem.zip"
# Intermediate files are not published.
Remove-Item -Force $icon
