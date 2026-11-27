param([string]$Binary = 'target\release\fero.exe')

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..\..')
$version = (Select-String -Path Cargo.toml -Pattern '^version = "([^"]+)"$').Matches[0].Groups[1].Value
foreach ($name in 'FERO_LIBTORRENT_PREFIX', 'FERO_NATIVE_DEPS', 'VCToolsRedistDir') {
    if (-not [Environment]::GetEnvironmentVariable($name)) { throw "Set $name before packaging" }
}
$headers = & dumpbin.exe /headers $Binary
if ($LASTEXITCODE -ne 0 -or -not ($headers -match '^\s*8664 machine')) { throw 'Expected an x64 Windows build' }
$crt = @(Get-ChildItem "$env:VCToolsRedistDir\x64" -Directory -Filter 'Microsoft.VC*.CRT')
if ($crt.Count -ne 1) { throw 'Expected one x64 Visual C++ CRT redistributable folder' }

$stage = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    Copy-Item -LiteralPath $Binary -Destination "$stage\fero.exe"
    foreach ($dir in "$env:FERO_LIBTORRENT_PREFIX\bin", "$env:FERO_NATIVE_DEPS\bin", $crt[0].FullName) {
        $dlls = @(Get-ChildItem -LiteralPath $dir -Filter '*.dll' -File)
        if (-not $dlls.Count) { throw "No runtime DLLs in $dir" }
        $dlls | Copy-Item -Destination $stage
    }
    New-Item -ItemType Directory -Path dist -Force | Out-Null
    $out = Join-Path (Resolve-Path dist) "Fero-$version-windows-x64.exe"
    & makensis.exe /WX "/DVERSION=$version" "/DSTAGE=$stage" "/DOUTPUT=$out" native/windows/Fero.nsi
    if ($LASTEXITCODE -ne 0) { throw "NSIS failed ($LASTEXITCODE)" }
} finally {
    Remove-Item -LiteralPath $stage -Recurse -Force
}
