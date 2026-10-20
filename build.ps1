# Compila kast.exe (CLI) y kastd.exe (bandeja + atajos + UI) con zig cc.
#   .\build.ps1                 versión "dev"
#   .\build.ps1 -Version 1.2.0  versión que muestran la interfaz y `kast version`
param([string]$Version = 'dev')

if ($Version -notmatch '^[0-9A-Za-z][0-9A-Za-z.\-]*$') { throw "versión inválida: '$Version' (sólo letras, números, . y -)" }
$zig = (Get-Command zig -ErrorAction SilentlyContinue).Source
if (-not $zig) {
    $zig = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Recurse -Filter zig.exe -ErrorAction SilentlyContinue |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $zig) { throw 'zig no encontrado: winget install zig.zig' }

Set-Location $PSScriptRoot
New-Item -ItemType Directory -Force bin | Out-Null
$flags = '-target', 'x86_64-windows-gnu', '-std=c11', '-Os', '-s', '-Wall', '-Wextra',
         '-ffunction-sections', '-fdata-sections', '-Wl,--gc-sections', '-Ithird_party/webview2', '-Wno-unknown-pragmas',
         "-DKAST_VERSION_RAW=$Version"
$libs = '-luser32', '-ldxva2', '-ladvapi32', '-lgdi32'

& $zig cc @flags src/core.c src/cli.c src/kast.rc @libs -o bin/kast.exe
if ($LASTEXITCODE) { exit $LASTEXITCODE }
& $zig cc @flags src/core.c src/daemon.c src/webui.c src/kastd.rc @libs `
    -lshell32 -lole32 -ldwmapi -lshcore '-Wl,--subsystem,windows' -o bin/kastd.exe
if ($LASTEXITCODE) { exit $LASTEXITCODE }

# Kast Link (Rust). Con el toolchain GNU de Rust hace falta el dlltool de MinGW-w64 en el PATH.
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo -and (Test-Path "$env:USERPROFILE\.cargo\bin\cargo.exe")) { $cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe" }
if ($cargo) {
    if (-not (Get-Command dlltool -ErrorAction SilentlyContinue)) {
        $mingw = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Recurse -Filter dlltool.exe -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($mingw) { $env:Path = "$($mingw.DirectoryName);$env:Path" }
    }
    & $cargo build --release --quiet --manifest-path link/Cargo.toml
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
    Copy-Item link/target/release/kast-link.exe bin/ -Force
} else {
    Write-Warning 'cargo no encontrado: se omite Kast Link (https://rustup.rs)'
}

Remove-Item bin/*.pdb, bin/*.lib -ErrorAction SilentlyContinue
Copy-Item third_party/webview2/WebView2Loader.dll bin/ -Force
if (-not (Test-Path bin/kast.ini)) { Copy-Item kast.ini bin/kast.ini }
Get-ChildItem bin/*.exe, bin/*.dll | ForEach-Object { '{0,-20} {1,7:N1} KB' -f $_.Name, ($_.Length / 1KB) }
