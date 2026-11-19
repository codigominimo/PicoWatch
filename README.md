# PicoWatch

Monitor de sistemas extremadamente ligero, escrito en Rust estable y sin dependencias externas. Funciona en **Windows y Linux**, en **x86 y ARM**.

**Estado:** Fase 4 completada (procesos y watchdog, con backends por sistema operativo). 57 pruebas unitarias en verde.

---

## 1. Filosofía y decisiones de diseño

| Principio | Decisión concreta |
| --- | --- |
| Tamaño mínimo | `opt-level = "z"`, LTO, una unidad de código y `strip`. |
| RAM mínima | Sin frameworks ni runtimes; un solo bloqueo de `stdout` por ejecución. |
| CPU en reposo | Todo es síncrono; `watch` duerme entre muestras y no crea hilos. |
| Cero dependencias | Sólo `std`, más declaraciones FFI directas. Ver §3. |
| Seguridad | Se conserva PIE/RELRO; no se usa `-no-pie` para ahorrar unos bytes. |
| Simplicidad | Sin `unwrap` en producción, sin estado global. El `unsafe` se limita a FFI justificada. |
| Portabilidad | Rust 1.61; siete objetivos verificados (§4). |

`panic = "abort"` reduce el binario y es seguro aquí porque no hay recursos que deban liberarse mediante desenrollado. Los errores de E/S se propagan y se informan por `stderr` con códigos de salida distintos.

### Matriz de plataformas

| Sistema | Arquitecturas | Backend | Limitación |
| --- | --- | --- | --- |
| Linux | x86_64, i686, aarch64, armv7 | `/proc`, `/sys`, `statvfs`, `sysconf` | Ninguna de las métricas implementadas. |
| Windows | x86_64, i686, aarch64 | `kernel32`, `iphlpapi`, Toolhelp32 | No hay API térmica ligera: `temperature` devuelve `Unsupported` y `status` muestra «no disponible en esta plataforma». El uso de CPU por proceso se muestra a `0.00%`. |

En Windows el porcentaje de CPU por núcleo no se ofrece: `GetSystemTimes` sólo da el agregado y la alternativa (PDH) añadiría una dependencia pesada. Tampoco se calcula CPU por proceso: exigiría abrir dos veces el handle de cada PID con `QueryProcessCycleTime`. En Linux ambos desgloses sí están disponibles y el porcentaje de proceso es relativo a un núcleo, como en `top`.

---

## 2. Arquitectura

```text
picowatch/
├── Cargo.toml
├── bench/
│   └── measure.ps1        # tamaño y dependencias de los 7 objetivos
├── src/
│   ├── main.rs            # punto de entrada, códigos de salida
│   ├── cli/
│   │   └── mod.rs         # Command, ParseError, parse() puro y testeable
│   ├── collectors/
│   │   ├── mod.rs
│   │   ├── cpu.rs         # /proc/stat | GetSystemTimes
│   │   ├── memory.rs      # /proc/meminfo | GlobalMemoryStatusEx
│   │   ├── disk.rs        # statvfs | GetDiskFreeSpaceExW
│   │   ├── temperature.rs # /sys/class/thermal (sysfs)
│   │   ├── network.rs     # /proc/net/dev | GetIfTable
│   │   └── processes.rs   # /proc/[pid]/stat | Toolhelp32
│   ├── core/
│   │   └── mod.rs         # NAME, VERSION (constantes compile-time)
│   ├── output/
│   │   ├── mod.rs
│   │   └── terminal.rs    # funciones Write genérico para todos los comandos
│   ├── platform/
│   │   ├── mod.rs
│   │   └── windows.rs     # FFI a kernel32, iphlpapi y Toolhelp32 (cfg windows)
│   └── watchdog/
│       └── mod.rs         # diff() puro + bucle run() con E/S inyectada
└── README.md
```

Árbol previsto, añadido sólo cuando la fase correspondiente lo justifique:

```text
src/
├── core/        config.rs, scheduler.rs, events.rs
├── storage/     mod.rs
└── output/      json.rs
```

Responsabilidades:

- **`cli`** son funciones puras sobre iteradores de `OsStr`: no leen `std::env` y se prueban sin efectos laterales. `parse` devuelve comando + argumentos; `parse_watch` valida las opciones.
- **`collectors`** separa el parseo puro (`parse_snapshot`, `parse_meminfo`, `parse_temperature`, `parse_netdev`, `parse_stat`) de la lectura del sistema. Las pruebas usan texto sintético y funcionan en cualquier host. Los porcentajes usan enteros en centésimas (`basis_points`), sin `f64`.
- **`watchdog`** calcula eventos con `observed` + `diff` sobre mapas ordenados; `run` recibe la lectura y el reposo como closures, así que las pruebas no duermen ni necesitan procesos reales. No reinicia nada: las alertas llegan en la Fase 5.
- **`platform`** concentra todo el código Windows específico. En Linux los colectores leen directamente con `std::fs`.
- **`output::terminal`** recibe `&mut impl Write`, así que las pruebas usan `Vec<u8>` y `main` hace un único `lock()` de `stdout`.
- **`main`** decide el código de salida: `0` éxito, `1` error de E/S, `2` error de sintaxis.

---

## 3. Dependencias y uso de `unsafe`

**Número de dependencias externas: 0** en los siete objetivos.

| Alternativa descartada | Problema que resolvía | Por qué no se usa |
| --- | --- | --- |
| `clap` | Parseo de CLI | Cientos de KiB y asignaciones para comandos fijos. |
| `serde` / `serde_json` | Salida JSON | Se implementará un escritor mínimo en la Fase 6. |
| `sysinfo` | Métricas del sistema | Arrastre de dependencias y allocations; `/proc`, `/sys` y la API nativa se usan directamente. |
| `libc` | Llamadas al sistema | Añadiría un crate y una capa que no evita declarar las estructuras (`statvfs`, `PROCESSENTRY32W`) ni las funciones de Windows. Se declaran las doce funciones necesarias. |
| `anyhow` / `thiserror` | Errores | Un enum de dos variantes y `From` bastan. |

Las funciones declaradas mediante FFI son `GetSystemTimes`, `GlobalMemoryStatusEx`, `GetDiskFreeSpaceExW`, `GetIfTable`, `CreateToolhelp32Snapshot`, `Process32FirstW`, `Process32NextW`, `OpenProcess`, `K32GetProcessMemoryInfo` y `CloseHandle` (Windows), y `statvfs` y `sysconf` (Linux). `std` no expone estas métricas y la alternativa de lanzar `wmic`/`tasklist`/`df`/PowerShell sería lenta, dependiente de la configuración regional y frágil. Cada bloque `unsafe` está acompañado de un comentario `SAFETY`; no hay `unsafe` en el parseo ni en la lógica de presentación. Los handles se cierran siempre, incluso cuando la consulta de memoria falla.

Criterio para incorporar cualquier dependencia futura: describir el problema, demostrar que implementarlo directamente no es fiable, medir el incremento de binario y de RAM, y documentarlo aquí antes del `Cargo.toml`.

---

## 4. Compilación

Perfil de release:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Objetivos verificados (todos compilan y enlazan):

| Sistema | Triple Cargo | Enlazador |
| --- | --- | --- |
| Windows x64 | `x86_64-pc-windows-msvc` | MSVC nativo |
| Windows x86 | `i686-pc-windows-msvc` | MSVC nativo |
| Windows ARM64 | `aarch64-pc-windows-msvc` | MSVC nativo |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `zig cc`, glibc estática |
| Linux i686 | `i686-unknown-linux-gnu` | `zig cc`, glibc estática |
| Linux AArch64 | `aarch64-unknown-linux-gnu` | `zig cc`, glibc estática |
| Linux ARMv7 | `armv7-unknown-linux-gnueabihf` | `zig cc`, glibc estática |

Desde Windows, el script mide cualquiera de ellos:

```powershell
.\bench\measure.ps1 -Target x86_64-pc-windows-msvc
.\bench\measure.ps1 -Target armv7-unknown-linux-gnueabihf
```

Escribe un envoltorio temporal para `zig cc`, fija el triple y `crt-static`, y filtra `--fix-cortex-a53-843419` / `--fix-cortex-a8`, que `rustc` añade para ARM y Zig rechaza. No modifica la configuración del repositorio; en una máquina Linux se compila con el toolchain natural:

```sh
cargo build --release --target x86_64-unknown-linux-gnu
cargo test --all-targets
```

La compilación estática se eligió para Linux porque el monitor no necesita NSS/DNS. Se reevaluará en la Fase 7 frente a la dinámica, que podría ser menor.

---

## 5. Uso

```text
picowatch                  Muestra la ayuda
picowatch help             Muestra la ayuda
picowatch -h | --help      Muestra la ayuda
picowatch version          Muestra la versión
picowatch -V | --version   Muestra la versión
picowatch status           Resumen general
picowatch cpu              Uso total y por núcleo
picowatch memory           RAM total, disponible y usada
picowatch disk             Espacio del volumen raíz
picowatch temperature      Sensores térmicos
picowatch network          Tráfico por interfaz de red
picowatch processes       Procesos: PID, nombre, hilos, RSS y CPU
picowatch watch [--interval MS] [--ticks N] nombre [nombre...]
                           Vigila procesos y notifica inicios y paradas
picowatch config          Comando reservado para la Fase 5
```

`network` y `watch` se añaden a la lista de ejemplos del enunciado para exponer de forma independiente los módulos de red y vigilancia. En `watch`, `--interval` vale 1000 ms por defecto y `--ticks 0` (por defecto) vigila hasta que se interrumpa el proceso.

Códigos de salida: `0` correcto, `1` error de E/S, `2` error de sintaxis. `cpu` y `processes` toman dos muestras separadas 100 ms; `watch` duerme entre muestras. Ningún comando crea hilos.

Ejemplo en Linux:

```console
$ picowatch status
PicoWatch - estado del sistema
CPU: 4.16%
Núcleos: 8
  cpu0: 3.10%
  ...
Memoria: 16384.0 MiB totales, 8192.0 MiB disponibles, 8192.0 MiB usadas (50.00%)
Disco (/): 476.9 GiB totales, 293.4 GiB disponibles, 183.4 GiB usadas (38.46%)
Temperatura: 1 sensor, máx. 45.2 °C
Red: 3 interfaces
```

Ejemplo en Windows:

```console
> picowatch temperature
picowatch: error de E/S: los sensores térmicos sólo están disponibles en Linux (sysfs)
> picowatch status
...
Temperatura: no disponible en esta plataforma
Red: 45 interfaces
> picowatch processes
Procesos: 267
    23744  opencode-cli.exe         420.4 MiB  27 hilos  0.00%
> picowatch watch --interval 50 --ticks 2 explorer.exe
Vigilando 1 proceso (cada 50 ms): explorer.exe
[watch] + iniciado: explorer.exe (pid 2816)
```

El `0.00%` de CPU por proceso es la limitación documentada de Windows; en Linux el valor se calcula comparando dos lecturas de `/proc/[pid]/stat`.

---

## 6. Medición

### Tamaños medidos

Entorno: Rust 1.99.1, Zig 0.17.1, perfil `release`, 19/11/2026.

| Fase | Objetivo | Tamaño | Δ fase anterior | Dependencias |
| --- | --- | ---: | ---: | ---: |
| 1 | Linux x86_64 | 301.856 B (294,8 KiB) | — | 0 |
| 1 | Linux AArch64 | 273.904 B (267,5 KiB) | — | 0 |
| 1 | Windows x64 | 117.760 B | — | 0 |
| 2 | Linux x86_64 | 313.928 B (306,6 KiB) | +12.072 B | 0 |
| 2 | Linux AArch64 | 285.120 B (278,4 KiB) | +11.216 B | 0 |
| 2 | Windows x64 | 139.776 B | +22.016 B | 0 |
| 3 | Linux x86_64 | 336.144 B (328,3 KiB) | +22.216 B | 0 |
| 3 | Linux i686 | 358.912 B (350,5 KiB) | nuevo | 0 |
| 3 | Linux AArch64 | 305.976 B (298,8 KiB) | +20.856 B | 0 |
| 3 | Linux ARMv7 | 328.624 B (320,9 KiB) | nuevo | 0 |
| 3 | Windows x64 | 136.192 B (133,0 KiB) | −3.584 B | 0 |
| 3 | Windows x86 | 127.488 B (124,5 KiB) | nuevo | 0 |
| 3 | Windows ARM64 | 131.584 B (128,5 KiB) | nuevo | 0 |
| 4 | **Linux x86_64** | **359.176 B (350,8 KiB)** | **+23.032 B** | 0 |
| 4 | **Linux i686** | **383.032 B (374,1 KiB)** | **+24.120 B** | 0 |
| 4 | **Linux AArch64** | **326.064 B (318,4 KiB)** | **+20.088 B** | 0 |
| 4 | **Linux ARMv7** | **347.320 B (339,2 KiB)** | **+18.696 B** | 0 |
| 4 | **Windows x64** | **153.600 B (150,0 KiB)** | **+17.408 B** | 0 |
| 4 | **Windows x86** | **144.384 B (141,0 KiB)** | **+16.896 B** | 0 |
| 4 | **Windows ARM64** | **147.968 B (144,5 KiB)** | **+16.384 B** | 0 |

El descenso de Windows x64 en la Fase 3 respecto a la Fase 2 vino de un refactor del formateo de porcentajes, no de un cambio de perfil. El incremento de la Fase 4 es el coste del colector de procesos y del watchdog. Todas las cifras siguen lejos del límite de 1-3 MB. **Ninguna optimización se declarará exitosa sin medición.**

### Procedimiento de RAM, CPU y arranque

Linux:

```sh
/usr/bin/time -v ./picowatch version
time (for i in $(seq 1 1000); do ./picowatch version > /dev/null; done)
time (for i in $(seq 1 100); do ./picowatch status > /dev/null; done)
/usr/bin/time -f "%e s, %U s usuario, %S s sistema, %M KiB RSS" ./picowatch status
```

Windows (PowerShell):

```powershell
Measure-Command { .\picowatch.exe status } | Select-Object TotalMilliseconds
```

Estas cifras se automatizarán en la Fase 7. En este entorno sólo se han ejecutado los binarios de Windows; los de Linux se generaron por enlazado cruzado y el subsistema WSL disponible no tenía su imagen montada, así que su ejecución queda pendiente de un equipo Linux.

---

## 7. Plan de desarrollo

| Fase | Contenido | Estado |
| --- | --- | --- |
| 1 | Estructura mínima, CLI y compilación optimizada | ✅ Completada |
| 2 | Monitorización de CPU y memoria | ✅ Completada |
| 3 | Discos, temperatura y red | ✅ Completada |
| 4 | Procesos y watchdog | ✅ Completada |
| 5 | Historial y alertas | ⏳ Pendiente |
| 6 | Salida JSON y pruebas | ⏳ Pendiente |
| 7 | Optimización extrema y benchmarking | ⏳ Pendiente |

No se avanza de fase hasta que la anterior funcione y tenga pruebas. La salida JSON se incorporará en la Fase 6; hasta entonces sólo existe salida legible.

---

## 8. Instalación

Linux:

```sh
install -m 0755 picowatch /usr/local/bin/picowatch
picowatch version
```

Windows: copiar `picowatch.exe` a un directorio incluido en `PATH` y ejecutar `picowatch version`.

No hay servicios, demonios ni archivos de configuración que instalar en esta fase.

## Licencia

MIT. Véase `LICENSE`.
