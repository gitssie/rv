param([ValidateSet('x86_64', 'aarch64')][string]$Architecture = 'x86_64')
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

# GitHub's Windows images supply MSVC/SDK, CMake, LLVM and Strawberry Perl.
$toolDirectories = @('C:\Strawberry\perl\bin', 'C:\Program Files\LLVM\bin', 'C:\Program Files\NASM')
foreach ($directory in $toolDirectories) {
    if (Test-Path $directory) {
        $env:PATH = "$directory;$env:PATH"
        Add-Content -Path $env:GITHUB_PATH -Value $directory
    }
}
if ($Architecture -eq 'x86_64' -and -not (Get-Command nasm -ErrorAction SilentlyContinue)) {
    choco install nasm --yes --no-progress
    if ($LASTEXITCODE -ne 0) { throw 'NASM installation failed' }
    $env:PATH = "C:\Program Files\NASM;$env:PATH"
    Add-Content -Path $env:GITHUB_PATH -Value 'C:\Program Files\NASM'
}
foreach ($tool in @('cargo', 'rustc', 'perl', 'cmake', 'clang', 'cl')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "Missing build tool: $tool" }
}
$compiler = 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe'
if (-not (Test-Path $compiler)) {
    choco install innosetup --yes --no-progress
    if ($LASTEXITCODE -ne 0) { throw 'Inno Setup installation failed' }
}
if (-not (Test-Path $compiler)) { throw 'Inno Setup compiler not found' }
# The package script embeds the icon; conversion is local to the GitHub runner.
python -m pip install --disable-pip-version-check Pillow
if ($LASTEXITCODE -ne 0) { throw 'Pillow installation failed' }
