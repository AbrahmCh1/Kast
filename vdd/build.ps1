# Compila el controlador de pantallas virtuales de Kast (KastDisplay) con el compilador de Visual Studio y el WDK de
# NuGet (packages.config), sin instalar el WDK. Deja el paquete del controlador en vdd\out\KastDisplay:
# KastDisplay.dll, KastDisplay.inf y, si se firma, KastDisplay.cat.
#
#   .\vdd\build.ps1 [-Version 1.2.3.0] [-Pfx firma.pfx -PfxPassword ...]
#
# Sin certificado se compila igual (para comprobar el código), pero Windows no instala un paquete sin firmar. La firma
# también se puede dar con las variables KAST_VDD_PFX (ruta o el .pfx en base64) y KAST_VDD_PFX_PASSWORD.
param(
    [string]$Version = '1.0.0.0',
    [string]$Pfx = $env:KAST_VDD_PFX,
    [string]$PfxPassword = $env:KAST_VDD_PFX_PASSWORD
)
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$pk = Join-Path $root 'packages'
$out = Join-Path $root 'out'
$pkg = Join-Path $out 'KastDisplay'

# El WDK y el SDK de Windows, de NuGet.
if (-not (Get-ChildItem $pk -Directory -Filter 'Microsoft.Windows.WDK.x64.*' -ErrorAction SilentlyContinue)) {
    $nuget = Join-Path $root 'tools\nuget.exe'
    if (-not (Test-Path $nuget)) {
        New-Item -ItemType Directory -Force (Split-Path $nuget) | Out-Null
        Invoke-WebRequest https://dist.nuget.org/win-x86-commandline/latest/nuget.exe -OutFile $nuget
    }
    & $nuget restore (Join-Path $root 'packages.config') -PackagesDirectory $pk -NonInteractive
    if ($LASTEXITCODE) { throw 'no se pudieron descargar los paquetes del WDK' }
}
function Pkg($filter) { (Get-ChildItem $pk -Directory -Filter $filter | Sort-Object Name | Select-Object -Last 1).FullName + '\c' }
$wdk = Pkg 'Microsoft.Windows.WDK.x64.*'
$sdk = Pkg 'Microsoft.Windows.SDK.CPP.1*'
$sdk64 = Pkg 'Microsoft.Windows.SDK.CPP.x64.*'
$kit = (Get-ChildItem "$sdk\Include" -Directory | Sort-Object Name | Select-Object -Last 1).Name
$umdf = '2.25'   # Windows 10 1803 o posterior
$iddcx = '1.4'   # Windows 10 1903 o posterior

# El compilador de C++ de Visual Studio.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'falta Visual Studio con el compilador de C++ (Desarrollo para el escritorio con C++)' }
$msvc = "$vs\VC\Tools\MSVC\" + (Get-Content "$vs\VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt").Trim()
$env:PATH = "$msvc\bin\Hostx64\x64;$env:PATH"
$env:INCLUDE = @("$msvc\include", "$sdk\Include\$kit\ucrt", "$sdk\Include\$kit\um", "$sdk\Include\$kit\shared", "$sdk\Include\$kit\winrt",
                 "$wdk\Include\$kit\um", "$wdk\Include\$kit\shared", "$wdk\Include\wdf\umdf\$umdf",
                 "$wdk\Include\$kit\um\iddcx\$iddcx") -join ';'
$env:LIB = @("$msvc\lib\x64", "$sdk64\um\x64", "$sdk64\ucrt\x64", "$wdk\Lib\wdf\umdf\x64\$umdf",
             "$wdk\Lib\$kit\um\x64\iddcx\$iddcx") -join ';'

Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $pkg | Out-Null
$obj = Join-Path $out 'obj'
New-Item -ItemType Directory -Force $obj | Out-Null

# Las versiones las pone el WDK al compilar con Visual Studio. Sin UMDF_VERSION_MINOR, el controlador le pide a Windows
# una versión mínima de UMDF imposible ((ULONG)-1, ver wdffuncenum.h) y Windows no lo carga (código 31).
$defs = '/DUMDF_DRIVER', '/DUMDF_VERSION_MAJOR=2', "/DUMDF_VERSION_MINOR=$($umdf.Split('.')[1])",
        '/DIDDCX_VERSION_MAJOR=1', "/DIDDCX_VERSION_MINOR=$($iddcx.Split('.')[1])", "/DIDDCX_MINIMUM_VERSION_REQUIRED=$($iddcx.Split('.')[1])",
        '/D_ATL_NO_WIN_SUPPORT', '/DUNICODE', '/D_UNICODE', '/DNDEBUG', '/D_WINDLL'
# Avisos como errores en nuestro código; los de las cabeceras de Windows, no. Con la biblioteca de C++ dentro (/MT): no
# depende de que el equipo tenga instalado el Visual C++ Redistributable.
& cl.exe /nologo /c /std:c++17 /O2 /MT /EHsc /W4 /WX /external:anglebrackets /external:W0 /GS /Gy /guard:cf /Zi /utf-8 @defs `
    "/Fo$obj\" "/Fd$obj\KastDisplay.pdb" `
    (Join-Path $root 'KastDisplay\Driver.cpp')
if ($LASTEXITCODE) { throw 'no compiló' }
& link.exe /nologo /DLL /SUBSYSTEM:WINDOWS /MACHINE:X64 /OPT:REF /OPT:ICF /GUARD:CF /DEBUG /DYNAMICBASE /NXCOMPAT `
    "/OUT:$pkg\KastDisplay.dll" "/PDB:$out\KastDisplay.pdb" "$obj\Driver.obj" `
    WdfDriverStubUm.lib IddCxStub.lib OneCoreUAP.lib avrt.lib ntdll.lib
if ($LASTEXITCODE) { throw 'no enlazó' }
Remove-Item "$pkg\KastDisplay.lib", "$pkg\KastDisplay.exp" -ErrorAction SilentlyContinue

# El .inf con su versión, la de UMDF y la arquitectura.
Copy-Item (Join-Path $root 'KastDisplay\KastDisplay.inf') $pkg
& "$wdk\bin\$kit\x64\stampinf.exe" -f "$pkg\KastDisplay.inf" -d '*' -a amd64 -v $Version -u "$umdf.0" | Out-Null
if ($LASTEXITCODE) { throw 'stampinf falló' }

if (-not $Pfx) {
    Write-Warning 'sin certificado: el paquete queda sin firmar (Windows no lo instala)'
    Get-ChildItem $pkg | ForEach-Object { '{0,-20} {1,8:N1} KB' -f $_.Name, ($_.Length / 1KB) }
    return
}
# Firma: la biblioteca, el catálogo del paquete (inf2cat) y el catálogo.
$pfxFile = $Pfx
if (-not (Test-Path $Pfx -ErrorAction SilentlyContinue)) {
    $pfxFile = Join-Path $out 'firma.pfx'
    [IO.File]::WriteAllBytes($pfxFile, [Convert]::FromBase64String($Pfx))
}
try {
    $signtool = "$sdk\bin\$kit\x64\signtool.exe"
    $sign = { param($f) & $signtool sign /q /fd sha256 /f $pfxFile /p $PfxPassword /tr http://timestamp.digicert.com /td sha256 $f
              if ($LASTEXITCODE) { throw "no se pudo firmar $f" } }
    & $sign "$pkg\KastDisplay.dll"
    & "$wdk\bin\$kit\x86\Inf2Cat.exe" /driver:$pkg /os:10_X64 /uselocaltime | Out-Null
    if ($LASTEXITCODE) { throw 'inf2cat falló' }
    & $sign "$pkg\KastDisplay.cat"
} finally {
    if ($pfxFile -ne $Pfx) { Remove-Item $pfxFile -Force -ErrorAction SilentlyContinue }
}
Get-ChildItem $pkg | ForEach-Object { '{0,-20} {1,8:N1} KB' -f $_.Name, ($_.Length / 1KB) }
