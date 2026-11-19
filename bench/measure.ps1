<#
.SYNOPSIS
  Mide tamaño y dependencias para cualquier objetivo soportado.

.DESCRIPTION
  Windows se compila con MSVC nativo. Linux se enlaza desde Windows con
  zig cc (libc estática) mediante un envoltorio temporal. El script no
  modifica la configuración del repositorio.

  RAM, CPU y tiempos se automatizan en Windows con .\bench\bench.ps1; en
  Linux se miden con /usr/bin/time según README.md.

.EXAMPLE
  .\bench\measure.ps1 -Target armv7-unknown-linux-gnueabihf
#>
param(
    [ValidateSet(
        'x86_64-pc-windows-msvc',
        'i686-pc-windows-msvc',
        'aarch64-pc-windows-msvc',
        'x86_64-unknown-linux-gnu',
        'i686-unknown-linux-gnu',
        'aarch64-unknown-linux-gnu',
        'armv7-unknown-linux-gnueabihf'
    )]
    [string]$Target = 'x86_64-unknown-linux-gnu'
)

# Windows PowerShell convierte stderr de comandos nativos en ErrorRecord;
# con 'Stop' abortaría aunque cargo haya terminado correctamente.
$ErrorActionPreference = 'Continue'

$zigTriples = @{
    'x86_64-unknown-linux-gnu'       = 'x86_64-linux-gnu'
    'i686-unknown-linux-gnu'         = 'x86-linux-gnu'
    'aarch64-unknown-linux-gnu'      = 'aarch64-linux-gnu'
    'armv7-unknown-linux-gnueabihf'  = 'arm-linux-gnueabihf'
}

$root = Split-Path -Parent $PSScriptRoot

# Limpia posibles valores de ejecuciones anteriores en la misma sesión.
foreach ($key in $zigTriples.Keys) {
    $name = $key.ToUpperInvariant().Replace('-', '_')
    [Environment]::SetEnvironmentVariable("CARGO_TARGET_${name}_LINKER", $null)
    [Environment]::SetEnvironmentVariable("CARGO_TARGET_${name}_RUSTFLAGS", $null)
}

if ($zigTriples.ContainsKey($Target)) {
    # rustc añade flags de corrección de erratas que zig cc no acepta.
    $wrapper = Join-Path ([System.IO.Path]::GetTempPath()) "picowatch-zig-cc-$Target.cmd"
    @(
        '@echo off',
        'setlocal EnableDelayedExpansion',
        'set "args="',
        ':loop',
        'if "%~1"=="" goto done',
        'if "%~1"=="-Wl,--fix-cortex-a53-843419" goto skip',
        'if "%~1"=="-Wl,--fix-cortex-a8" goto skip',
        'set "args=!args! %1"',
        ':skip',
        'shift',
        'goto loop',
        ':done',
        'zig cc %args%'
    ) | Set-Content -Path $wrapper -Encoding ASCII

    $envName = $Target.ToUpperInvariant().Replace('-', '_')
    [Environment]::SetEnvironmentVariable("CARGO_TARGET_${envName}_LINKER", $wrapper)
    [Environment]::SetEnvironmentVariable(
        "CARGO_TARGET_${envName}_RUSTFLAGS",
        "-C link-arg=-target -C link-arg=$($zigTriples[$Target]) -C target-feature=+crt-static"
    )
}

Push-Location $root
try {
    cargo build --release --target $Target 2>&1 | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "cargo build falló con código $LASTEXITCODE" }

    $exeName = if ($Target -like '*windows*') { 'picowatch.exe' } else { 'picowatch' }
    $exe = Join-Path $root "target/$Target/release/$exeName"
    $bytes = (Get-Item -LiteralPath $exe).Length
    $lines = @(cargo tree --edges normal --prefix none)
    $deps = [Math]::Max(0, $lines.Count - 1)
    $link = if ($zigTriples.ContainsKey($Target)) { 'estático, glibc vía zig cc' } else { 'MSVC nativo' }

    Write-Host "Objetivo:      $Target ($link)"
    Write-Host "Tamaño:        $bytes bytes ($([Math]::Round($bytes / 1KB, 1)) KiB)"
    Write-Host "Dependencias:  $deps (excluyendo el propio paquete)"
    Write-Host "Binario:       $exe"
}
finally {
    Pop-Location
}
