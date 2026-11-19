<#
.SYNOPSIS
  Mide arranque, RAM, CPU y tiempo de los colectores de PicoWatch en Windows.

.DESCRIPTION
  Lanza el ejecutable con ProcessStartInfo y salida redirigida, por lo que no
  depende de la consola ni de Measure-Command. La medición de tiempo no sondea
  el proceso; la de RAM hace una pasada separada porque PeakWorkingSet64 vuelve
  a 0 después de WaitForExit y hay que leerlo mientras el proceso vive. Todo
  usa .NET y PowerShell, sin módulos ni crates extra.

  Los comandos `cpu`, `processes` y `status` incluyen una espera deliberada de
  100 ms entre dos muestras: su tiempo de pared no es coste de cómputo.

  Los binarios de Linux no se pueden ejecutar desde Windows: para medirlos,
  copia este script y el ejecutable a un equipo Linux y usa /usr/bin/time
  según README.md.

.EXAMPLE
  .\bench\bench.ps1 -Iterations 30
#>
param(
    [string]$Exe = '',
    [ValidateRange(1, 1000)]
    [int]$Iterations = 20
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
if (-not $Exe) {
    $Exe = Join-Path $root 'target/x86_64-pc-windows-msvc/release/picowatch.exe'
}
if (-not (Test-Path -LiteralPath $Exe)) {
    throw "No existe el ejecutable: $Exe"
}
$Exe = (Resolve-Path -LiteralPath $Exe).Path

function Invoke-Measured {
    param(
        [string[]]$Arguments,
        [switch]$MeasureRam
    )

    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Exe
    $start.Arguments = ($Arguments -join ' ')
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $process = [System.Diagnostics.Process]::Start($start)
    $peak = [int64]0

    if ($MeasureRam) {
        # Lectura asíncrona: un proceso con mucha salida no debe bloquearse
        # mientras sondeamos su memoria.
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        while (-not $process.HasExited -and $watch.Elapsed.TotalMilliseconds -lt 10000) {
            $process.Refresh()
            try {
                if ($process.PeakWorkingSet64 -gt $peak) { $peak = $process.PeakWorkingSet64 }
                if ($process.WorkingSet64 -gt $peak) { $peak = $process.WorkingSet64 }
            } catch { }
        }
        $process.WaitForExit()
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
    } else {
        $stdout = $process.StandardOutput.ReadToEnd()
        $stderr = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
    }
    $watch.Stop()

    [pscustomobject]@{
        WallMs = $watch.Elapsed.TotalMilliseconds
        CpuMs  = ($process.UserProcessorTime + $process.PrivilegedProcessorTime).TotalMilliseconds
        RamKib = [Math]::Round($peak / 1KB, 1)
        Exit   = $process.ExitCode
        Bytes  = $stdout.Length
        Error  = $stderr.Trim()
    }
}

function Median([double[]]$Values) {
    $sorted = @($Values | Sort-Object)
    $middle = [int][Math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return $sorted[$middle] }
    return ($sorted[$middle - 1] + $sorted[$middle]) / 2
}

Write-Host "Ejecutable:   $Exe"
Write-Host "Iteraciones:  $Iterations (tiempo) + 3 (RAM)"
Write-Host ""

# Arranque: el comando más barato, sin E/S de sistema ni archivos.
$startup = @(
    for ($i = 0; $i -lt $Iterations; $i++) { Invoke-Measured @('version') }
)
$ram = @(
    for ($i = 0; $i -lt 3; $i++) { Invoke-Measured @('version') -MeasureRam }
)
Write-Host "ARRANQUE (version)"
Write-Host ("  pared:    mediana {0:N2} ms | min {1:N2} ms" -f (Median ([double[]]$startup.WallMs)), (($startup.WallMs | Measure-Object -Minimum).Minimum))
Write-Host ("  CPU:      mediana {0:N2} ms (el reloj de Windows tiene granularidad de ~15,6 ms)" -f (Median ([double[]]$startup.CpuMs)))
Write-Host ("  RAM pico: mediana {0:N1} KiB" -f (Median ([double[]]$ram.RamKib)))
Write-Host ("  salida:   {0} bytes | exit {1}" -f $startup[0].Bytes, $startup[0].Exit)
Write-Host ""

# Un lanzamiento para tiempo y otro para RAM por comando.
$commands = @(
    @{ Name = 'help';         Args = @('help') },
    @{ Name = 'version';      Args = @('version') },
    @{ Name = 'cpu';          Args = @('cpu') },
    @{ Name = 'memory';       Args = @('memory') },
    @{ Name = 'disk';         Args = @('disk') },
    @{ Name = 'temperature';  Args = @('temperature') },
    @{ Name = 'network';      Args = @('network') },
    @{ Name = 'processes';    Args = @('processes') },
    @{ Name = 'status';       Args = @('status') },
    @{ Name = 'config';       Args = @('config', 'path') },
    @{ Name = 'history';      Args = @('history', '--last', '5') },
    @{ Name = 'status --json'; Args = @('--json', 'status') }
)
Write-Host "COLECTORES (cpu/processes/status duermen 100 ms entre muestras)"
Write-Host ("{0,-16} {1,11} {2,10} {3,11} {4,8} {5,6}" -f 'Comando', 'Pared ms', 'CPU ms', 'RAM KiB', 'Salida', 'Exit')
foreach ($case in $commands) {
    $m = Invoke-Measured $case.Args
    $r = Invoke-Measured $case.Args -MeasureRam
    Write-Host ("{0,-16} {1,11:N2} {2,10:N2} {3,11:N1} {4,8} {5,6}" -f $case.Name, $m.WallMs, $m.CpuMs, $r.RamKib, $m.Bytes, $m.Exit)
    if ($m.Error) { Write-Host ("  stderr: {0}" -f $m.Error) }
}
