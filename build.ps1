# Compila kast.exe (CLI) y kastd.exe (bandeja + atajos + UI) con zig cc.
$zig = (Get-Command zig -ErrorAction SilentlyContinue).Source
if (-not $zig) {
    $zig = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Recurse -Filter zig.exe |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $zig) { throw 'zig no encontrado: winget install zig.zig' }

Set-Location $PSScriptRoot
New-Item -ItemType Directory -Force bin | Out-Null
$flags = '-target', 'x86_64-windows-gnu', '-std=c11', '-Os', '-s', '-Wall', '-Wextra',
         '-ffunction-sections', '-fdata-sections', '-Wl,--gc-sections', '-Ithird_party/webview2', '-Wno-unknown-pragmas'
$libs = '-luser32', '-ldxva2', '-ladvapi32', '-lgdi32'

& $zig cc @flags src/core.c src/cli.c src/kast.rc @libs -o bin/kast.exe
if ($LASTEXITCODE) { exit $LASTEXITCODE }
& $zig cc @flags src/core.c src/daemon.c src/webui.c src/kastd.rc @libs `
    -lshell32 -lole32 -ldwmapi -lshcore '-Wl,--subsystem,windows' -o bin/kastd.exe
if ($LASTEXITCODE) { exit $LASTEXITCODE }

Remove-Item bin/*.pdb, bin/*.lib -ErrorAction SilentlyContinue
Copy-Item third_party/webview2/WebView2Loader.dll bin/ -Force
if (-not (Test-Path bin/kast.ini)) { Copy-Item kast.ini bin/kast.ini }
Get-ChildItem bin/*.exe, bin/*.dll | ForEach-Object { '{0,-20} {1,7:N1} KB' -f $_.Name, ($_.Length / 1KB) }
