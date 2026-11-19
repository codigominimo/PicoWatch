# PicoWatch

Monitor de sistemas extremadamente ligero, escrito en Rust estable. Funciona en **Windows y Linux**, en **x86 y ARM**. Al ejecutarlo sin argumentos abre un **panel interactivo con gráficas**; los comandos de texto y JSON siguen disponibles para guiones.

**Estado:** Fase 9 completada (interfaz TUI con ratatui). 101 pruebas unitarias en verde. Los colectores y la CLI siguen sin dependencias; el panel añade dos crates (§3).

---

## 1. Filosofía y decisiones de diseño

| Principio | Decisión concreta |
| --- | --- |
| Tamaño mínimo | `opt-level = "z"`, LTO, una unidad de código y `strip`. |
| RAM mínima | Sin runtimes ni hilos; un solo bloqueo de `stdout` por comando. El TUI mantiene 60 muestras por gráfica (~2 KB). |
| CPU en reposo | Todo es síncrono; `watch` y el TUI duermen entre muestras y no crean hilos. |
| Dependencias | Colectores, CLI, JSON y watchdog: sólo `std` + FFI directa. El panel usa `ratatui` y `crossterm` (§3). |
| Seguridad | Se conserva PIE/RELRO; no se usa `-no-pie` para ahorrar unos bytes. |
| Simplicidad | Sin `unwrap` en producción, sin estado global. El `unsafe` se limita a FFI justificada. |
| Portabilidad | Rust 1.74 (MSRV de ratatui); siete objetivos verificados (§4). |

`panic = "abort"` reduce el binario y es seguro aquí porque no hay recursos que deban liberarse mediante desenrollado. Los errores de E/S se propagan y se informan por `stderr` con códigos de salida distintos.

### Matriz de plataformas

| Sistema | Arquitecturas | Backend | Limitación |
| --- | --- | --- | --- |
| Linux | x86_64, i686, aarch64, armv7 | `/proc`, `/sys`, `statvfs`, `sysconf` | Ninguna de las métricas implementadas. |
| Windows | x86_64, i686, aarch64 | `kernel32`, `iphlpapi`, `ntdll`, Toolhelp32 | Sin sensores térmicos: la única fuente es WMI/COM. `temperature` informa «sin sensores disponibles» y `status` omite la sección de alertas térmicas. |

En Windows **sí** se ofrece el desglose de CPU por núcleo y por proceso (Fase 8): `GetSystemTimes` sólo da el agregado, así que los núcleos se leen con la clase 8 de `NtQuerySystemInformation` (`winternl.h`, estable desde Windows NT) y el tiempo por proceso con `GetProcessTimes`, tomando dos muestras como en Linux. No se usa PDH: añadiría un query, rutas de contador sensibles al idioma y aritmética de coma flotante. El porcentaje de proceso es relativo a un núcleo, igual que en `top`. Para la temperatura no existe API ligera equivalente a sysfs: WMI/COM exigiría cientos de líneas y una dependencia de runtime que no se puede validar en este entorno, así que se devuelve una lista vacía en lugar de un error.

---

## 2. Arquitectura

```text
picowatch/
├── Cargo.toml
├── bench/
│   ├── measure.ps1        # tamaño y dependencias de los 7 objetivos
│   └── bench.ps1          # arranque, RAM, CPU y tiempos por comando (Windows)
├── src/
│   ├── main.rs            # punto de entrada, códigos de salida
│   ├── alerts/
│   │   └── mod.rs         # evaluate() puro de umbrales
│   ├── cli/
│   │   └── mod.rs         # Command, ParseError, parse() puro y testeable
│   ├── collectors/
│   │   ├── mod.rs
│   │   ├── cpu.rs         # /proc/stat | GetSystemTimes + NtQuerySystemInformation
│   │   ├── memory.rs      # /proc/meminfo | GlobalMemoryStatusEx
│   │   ├── disk.rs        # statvfs | GetDiskFreeSpaceExW
│   │   ├── temperature.rs # /sys/class/thermal (sysfs); vacío en Windows
│   │   ├── network.rs     # /proc/net/dev | GetIfTable
│   │   └── processes.rs   # /proc/[pid]/stat | Toolhelp32 + GetProcessTimes
│   ├── config/
│   │   └── mod.rs         # clave=valor, rutas por plataforma
│   ├── core/
│   │   └── mod.rs         # NAME, VERSION (constantes compile-time)
│   ├── output/
│   │   ├── mod.rs
│   │   ├── json.rs        # escritor JSON mínimo + StatusView
│   │   └── terminal.rs    # funciones Write genérico para todos los comandos
│   ├── platform/
│   │   ├── mod.rs
│   │   └── windows.rs     # FFI a kernel32, iphlpapi, ntdll y Toolhelp32 (cfg windows)
│   ├── storage/
│   │   └── mod.rs         # historial TSV acotado
│   ├── tui/
│   │   └── mod.rs         # panel ratatui: gauges, sparklines, barras y tabla
│   └── watchdog/
│       └── mod.rs         # diff() puro + bucle run() con E/S inyectada
└── README.md
```

Responsabilidades:

- **`cli`** son funciones puras sobre iteradores de `OsStr`: no leen `std::env` y se prueban sin efectos laterales. `parse` devuelve comando + argumentos + el flag `--json` (que admite antes o después del comando); `parse_watch`, `parse_config` y `parse_history` validan las opciones.
- **`collectors`** separa el parseo puro (`parse_snapshot`, `parse_meminfo`, `parse_temperature`, `parse_netdev`, `parse_stat`) de la lectura del sistema. Las pruebas usan texto sintético y funcionan en cualquier host. Los porcentajes usan enteros en centésimas (`basis_points`), sin `f64`.
- **`config`** parsea `clave=valor` con `parse()` puro; `load()` usa los valores por defecto si no hay archivo. La ruta depende de `APPDATA` (Windows) o `XDG_CONFIG_HOME`/`HOME` (Linux), con retroceso al directorio actual.
- **`storage`** guarda el historial en TSV, con `format_line`/`parse_line` puros y `append` acotado a `history_max` líneas. Una línea corrupta se ignora sin bloquear las muestras nuevas.
- **`alerts`** es una función pura que compara métricas con umbrales; no imprime ni lee archivos.
- **`watchdog`** calcula eventos con `observed` + `diff` sobre mapas ordenados; `run` recibe la lectura, el reposo y el emisor de eventos como closures, así que las pruebas no duermen ni necesitan procesos reales, y el mismo bucle sirve para texto y para JSON.
- **`platform`** concentra todo el código Windows específico. En Linux los colectores leen directamente con `std::fs`.
- **`output::terminal`** y **`output::json`** reciben `&mut impl Write`, así que las pruebas usan `Vec<u8>` y `main` hace un único `lock()` de `stdout`. El escritor JSON escapa cadenas y emite porcentajes como enteros; no usa `serde`.
- **`tui`** es el único módulo con dependencias externas. `App` concentra el estado de la pantalla (última muestra, históricos acotados, intervalo y pausa); `run` activa la pantalla alternativa, dibuja, sondea eventos 100 ms y restaura la terminal con un guardián `Drop`. Las pruebas dibujan con `TestBackend`, sin terminal real.
- **`main`** decide el código de salida: `0` éxito, `1` error de E/S, `2` error de sintaxis. Con `tui` activo toma posesión de la terminal antes de bloquear `stdout`.

---

## 3. Dependencias y uso de `unsafe`

**Dependencias externas: 2 directas** (`ratatui 0.29`, `crossterm 0.28`) en los siete objetivos. Hasta la Fase 8 eran cero; la Fase 9 las incorpora de forma explícita para no reescribir un motor de terminal, y documenta el coste medido en §6.

| Alternativa descartada | Problema que resolvía | Por qué no se usa |
| --- | --- | --- |
| `clap` | Parseo de CLI | Cientos de KiB y asignaciones para comandos fijos. |
| `serde` / `serde_json` | Salida JSON | Se resolvió en la Fase 6 con `output::json`: ~7-12 KB frente a cientos de KiB. |
| `sysinfo` | Métricas del sistema | Arrastre de dependencias y allocations; `/proc`, `/sys` y la API nativa se usan directamente. |
| `libc` | Llamadas al sistema | Añadiría un crate y una capa que no evita declarar las estructuras (`statvfs`, `PROCESSENTRY32W`) ni las funciones de Windows. Se declaran las catorce funciones necesarias. |
| `anyhow` / `thiserror` | Errores | Un enum de dos variantes y `From` bastan. |
| Dibujar la TUI con `std` | Panel con gráficas | Escapes ANSI, layouts, tablas, UTF-8 y eventos de teclado multiplataforma son cientos de líneas frágiles; ratatui + crossterm cuestan ~204 KB (Windows x64) y son la única dependencia nueva. |

Las funciones declaradas mediante FFI son `GetSystemTimes`, `GetProcessTimes`, `GlobalMemoryStatusEx`, `GetDiskFreeSpaceExW`, `GetIfTable`, `CreateToolhelp32Snapshot`, `Process32FirstW`, `Process32NextW`, `OpenProcess`, `K32GetProcessMemoryInfo`, `CloseHandle` y `NtQuerySystemInformation` (Windows), y `statvfs` y `sysconf` (Linux). Todas son API del sistema: no se añade ninguna DLL externa. `std` no expone estas métricas y la alternativa de lanzar `wmic`/`tasklist`/`df`/PowerShell sería lenta, dependiente de la configuración regional y frágil. Cada bloque `unsafe` está acompañado de un comentario `SAFETY`; no hay `unsafe` en el parseo ni en la lógica de presentación. Los handles se cierran siempre, incluso cuando falla alguna de las dos consultas por proceso, que comparten un único `OpenProcess`.

Criterio para incorporar cualquier dependencia futura: describir el problema, demostrar que implementarlo directamente no es fiable, medir el incremento de binario y de RAM, y documentarlo aquí antes del `Cargo.toml`. La Fase 9 siguió exactamente ese criterio: el TUI se dibuja y se prueba con `TestBackend` antes de medir los siete binarios (§6). `cargo tree -e normal --depth 1` confirma que sólo hay dos dependencias directas.

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

La compilación estática se eligió para Linux porque el monitor no necesita NSS/DNS; la medición de la Fase 7 confirmó que era la opción correcta y se conserva en la Fase 9.

---

## 5. Uso

```text
picowatch                  Abre el panel interactivo (TUI) con gráficas
picowatch tui | ui         Lo mismo, explícitamente
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
picowatch processes        Procesos: PID, nombre, hilos, RSS y CPU
picowatch watch [--interval MS] [--ticks N] nombre [nombre...]
                           Vigila procesos y notifica inicios y paradas
picowatch config [show|path|init]
                           Muestra la configuración, su ruta o la crea
picowatch history [--last N]
                           Últimas N muestras (por defecto 10)
picowatch [--json] comando  Emite JSON en lugar de texto legible
```

En el panel, `q` o `Esc` salen, `+`/`-` ajustan el intervalo (0,5-30 s), `r` fuerza un refresco y la barra espaciadora pausa. Los gráficos son histogramas de CPU (sparkline +gauge), uso por núcleo (barras), memoria, disco y tráfico RX/TX; debajo están el top de procesos y las alertas. Todo se dibuja con los mismos colectores que `status`, sin hilos.

`network`, `watch` e `history` se añaden a la lista de ejemplos del enunciado para exponer de forma independiente los módulos de red, vigilancia e historial. En `watch`, `--interval` vale 1000 ms por defecto y `--ticks 0` (por defecto) vigila hasta que se interrumpa el proceso.

`--json` es global: se acepta antes o después del comando (`picowatch --json cpu` o `picowatch cpu --json`) y afecta a todos los comandos. Los porcentajes se emiten como puntos base enteros (`5000` = 50,00 %), sin coma flotante. `watch` usa JSON Lines: una línea para la cabecera y una por evento.

`status` escribe una muestra en el historial y evalúa las alertas. El archivo de configuración se crea con `config init`; hasta entonces se usan los valores por defecto. En Windows queda en `%APPDATA%\PicoWatch\config.txt` y en Linux en `$XDG_CONFIG_HOME/picowatch/config` (o `~/.config/picowatch/config`). El historial se guarda como `history.log` junto a él, o en la ruta que indique `history_path`. `history_enabled=false` desactiva la escritura.

Códigos de salida: `0` correcto, `1` error de E/S, `2` error de sintaxis. `cpu` y `processes` toman dos muestras separadas 100 ms en ambas plataformas (Fase 8); `watch` duerme entre muestras. Ningún comando crea hilos.

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
Alertas: 1
  Memoria: 95.00% (umbral 90.00%)
$ picowatch history --last 2
Historial (2 muestras):
  1795128379 CPU 8.33% MEM 95.00% DISCO 38.51% RED 3 TEMP 45.2 °C
  1795128380 CPU 4.16% MEM 94.80% DISCO 38.51% RED 3 TEMP 45.1 °C
```

El mismo `status` con `--json`:

```console
$ picowatch --json status
{"cpu":{"usage_basis_points":416,"core_count":2,"cores":[310,520]},"memory":{"total_bytes":17179869184,"available_bytes":8589934592,"used_bytes":8589934592,"used_basis_points":5000},"disk":{"volume":"/","total_bytes":512000000000,"available_bytes":315000000000,"used_bytes":197000000000,"used_basis_points":3846},"network":{"interfaces":[{"name":"lo","rx_bytes":12345,"tx_bytes":67890},{"name":"eth0","rx_bytes":1000000,"tx_bytes":2000000}]},"temperature":{"sensors":[{"name":"thermal_zone0","millicelsius":45200}]},"alerts":[]}
```

Ejemplo en Windows:

```console
> picowatch temperature
Temperatura: sin sensores disponibles
> picowatch cpu
CPU: 5.40%
Núcleos: 12
  cpu0: 0.00%
  cpu1: 0.00%
  cpu2: 16.66%
  ...
> picowatch status
...
Temperatura: sin sensores disponibles
Red: 45 interfaces
> picowatch processes
Procesos: 268
     6016  msedge.exe               179.6 MiB  16 hilos  62.50%
    12556  msedge.exe               148.6 MiB  48 hilos  31.25%
    17724  OpenCode.exe             107.3 MiB  43 hilos  31.25%
> picowatch watch --interval 50 --ticks 2 explorer.exe
Vigilando 1 proceso (cada 50 ms): explorer.exe
[watch] + iniciado: explorer.exe (pid 2816)
> picowatch config path
C:\Users\Derte\AppData\Roaming\PicoWatch\config.txt
```

El desglose por núcleo y el porcentaje de proceso ya funcionan en Windows: se
calculan con dos lecturas de contadores acumulativos (`NtQuerySystemInformation`
y `GetProcessTimes`) y se expresan en puntos base relativos a un núcleo, igual
que en Linux. La única limitación restante es la temperatura: Windows no expone
una API ligera y se informa de «sin sensores disponibles» sin provocar un error
de E/S.

---

## 6. Medición

### Tamaños medidos

Entorno: Rust 1.99.1, Zig 0.17.1, perfil `release`, 20/11/2026.

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
| 4 | Linux x86_64 | 359.176 B (350,8 KiB) | +23.032 B | 0 |
| 4 | Linux i686 | 383.032 B (374,1 KiB) | +24.120 B | 0 |
| 4 | Linux AArch64 | 326.064 B (318,4 KiB) | +20.088 B | 0 |
| 4 | Linux ARMv7 | 347.320 B (339,2 KiB) | +18.696 B | 0 |
| 4 | Windows x64 | 153.600 B (150,0 KiB) | +17.408 B | 0 |
| 4 | Windows x86 | 144.384 B (141,0 KiB) | +16.896 B | 0 |
| 4 | Windows ARM64 | 147.968 B (144,5 KiB) | +16.384 B | 0 |
| 5 | Linux x86_64 | 379.656 B (370,8 KiB) | +20.480 B | 0 |
| 5 | Linux i686 | 410.176 B (400,6 KiB) | +27.144 B | 0 |
| 5 | Linux AArch64 | 344.344 B (336,3 KiB) | +18.280 B | 0 |
| 5 | Linux ARMv7 | 371.460 B (362,8 KiB) | +24.140 B | 0 |
| 5 | Windows x64 | 190.976 B (186,5 KiB) | +37.376 B | 0 |
| 5 | Windows x86 | 181.248 B (177,0 KiB) | +36.864 B | 0 |
| 5 | Windows ARM64 | 181.760 B (177,5 KiB) | +33.792 B | 0 |
| 6 | Linux x86_64 | 388.824 B (379,7 KiB) | +9.168 B | 0 |
| 6 | Linux i686 | 422.624 B (412,7 KiB) | +12.448 B | 0 |
| 6 | Linux AArch64 | 352.696 B (344,4 KiB) | +8.352 B | 0 |
| 6 | Linux ARMv7 | 381.492 B (372,6 KiB) | +10.032 B | 0 |
| 6 | Windows x64 | 199.168 B (194,5 KiB) | +8.192 B | 0 |
| 6 | Windows x86 | 189.440 B (185,0 KiB) | +8.192 B | 0 |
| 6 | Windows ARM64 | 188.928 B (184,5 KiB) | +7.168 B | 0 |
| 7 | Linux x86_64 | 389.040 B (380,1 KiB) | +216 B | 0 |
| 7 | Linux i686 | 422.848 B (412,9 KiB) | +224 B | 0 |
| 7 | Linux AArch64 | 352.960 B (344,7 KiB) | +264 B | 0 |
| 7 | Linux ARMv7 | 382.036 B (373,1 KiB) | +544 B | 0 |
| 7 | Windows x64 | 199.168 B (194,5 KiB) | 0 B | 0 |
| 7 | Windows x86 | 189.440 B (185,0 KiB) | 0 B | 0 |
| 7 | Windows ARM64 | 188.928 B (184,5 KiB) | 0 B | 0 |
| 8 | Linux x86_64 | 389.232 B (380,1 KiB) | +192 B | 0 |
| 8 | Linux i686 | 422.848 B (412,9 KiB) | 0 B | 0 |
| 8 | Linux AArch64 | 352.944 B (344,7 KiB) | −16 B | 0 |
| 8 | Linux ARMv7 | 381.956 B (373,0 KiB) | −80 B | 0 |
| 8 | Windows x64 | 199.680 B (195,0 KiB) | +512 B | 0 |
| 8 | Windows x86 | 188.928 B (184,5 KiB) | −512 B | 0 |
| 8 | Windows ARM64 | 189.440 B (185,0 KiB) | +512 B | 0 |
| 9 | **Linux x86_64** | **671.632 B (656,1 KiB)** | **+282.400 B** | 2 |
| 9 | **Linux i686** | **738.656 B (721,5 KiB)** | **+315.808 B** | 2 |
| 9 | **Linux AArch64** | **614.304 B (599,9 KiB)** | **+261.360 B** | 2 |
| 9 | **Linux ARMv7** | **658.364 B (643,0 KiB)** | **+276.408 B** | 2 |
| 9 | **Windows x64** | **403.968 B (394,5 KiB)** | **+204.288 B** | 2 |
| 9 | **Windows x86** | **393.728 B (384,5 KiB)** | **+204.800 B** | 2 |
| 9 | **Windows ARM64** | **386.048 B (377,0 KiB)** | **+196.608 B** | 2 |

El descenso de Windows x64 en la Fase 3 respecto a la Fase 2 vino de un refactor del formateo de porcentajes, no de un cambio de perfil. La Fase 4 añadió el colector de procesos y el watchdog; la Fase 5, configuración, historial y alertas; la Fase 6, el escritor JSON; la Fase 8, el desglose de CPU por núcleo y por proceso en Windows; la Fase 9, el panel con gráficas. El coste medido del TUI es de ~0,2-0,3 MB (un 70-100 % sobre la Fase 8), pero los siete binarios siguen por debajo de 750 KB y muy lejos del límite de 1-3 MB del proyecto. Los comandos de texto no pagan ningún coste de ejecución adicional: ratatui sólo se inicializa al abrir el panel. **Ninguna optimización se declarará exitosa sin medición.**

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
.\bench\bench.ps1 -Iterations 30
```

`bench.ps1` lanza cada comando con `ProcessStartInfo` y salida redirigida. Mide el tiempo de pared y de CPU sin sondear el proceso, y hace una pasada separada para el pico de `WorkingSet64` (Windows lo pone a 0 después de `WaitForExit`). No necesita módulos externos.

Resultados reales en este entorno, Windows x64, 30 iteraciones:

| Comando | Pared (ms) | CPU (ms) | RAM pico (KiB) | Salida |
| --- | ---: | ---: | ---: | ---: |
| `version` (arranque) | 28,06 (mediana; mín. 26,36) | 15,63 | 3.472 | 16 B |
| `help` | 29,92 | 0,00 | 3.048 | 1.255 B |
| `cpu` (12 núcleos) | 130,31 | 0,00 | 3.988 | 198 B |
| `memory` | 31,32 | 15,63 | 4.460 | 81 B |
| `disk` | 31,53 | 0,00 | 3.728 | 79 B |
| `temperature` | 31,33 | 0,00 | 3.212 | 38 B, exit 0 |
| `network` | 32,71 | 31,25 | 4.260 | 3.568 B |
| `processes` (268 proc.) | 153,92 | 31,25 | 4.748 | 16.678 B |
| `status` | 136,40 | 0,00 | 4.128 | 461 B |
| `status --json` | 133,71 | 0,00 | 4.132 | 4.567 B |

El reloj de CPU de Windows tiene una granularidad de ~15,6 ms, de ahí los valores discretos. `cpu`, `processes` y `status` incluyen la espera deliberada de 100 ms entre muestras; el sobrecoste de JSON frente a `status` legible no es medible y añade 4 KB de salida. La RAM son ~3-5 MB de huella del proceso Windows (CRT, pila y las tablas de ratatui enlazadas), no del heap de la aplicación: subió ~0,8 MB respecto a la Fase 8 por las dos dependencias nuevas, incluso en comandos que no las usan, porque el enlazador las incluye en el mismo ejecutable. El panel en ejecución se midió aparte: tras 1,5 s seguía activo y refrescando, con 12 gráficas de núcleo, sparklines de red y el top de procesos.

Decisiones de la Fase 9, verificadas con medición:

- `ratatui 0.29` + `crossterm 0.28` son las únicas dependencias: ~205 KB en Windows x64 y ~0,26-0,32 MB en Linux. Se acepta el coste a cambio de no mantener un motor de terminal propio.
- El panel no crea hilos: dos muestras de 100 ms por refrescado (CPU y procesos) y `event::poll(100 ms)`. El intervalo por defecto es 1 s y se ajusta de 0,5 a 30 s con `+`/`-`.
- El histórico se acota a 60 muestras por gráfica con `VecDeque`, y las pruebas de render usan `TestBackend`: el panel se verifica sin terminal real ni escribir a mano secuencias ANSI.
- La opción por defecto pasó de «ayuda» a «panel», porque la petición fue explícita: no hay que escribir comandos para ver el monitor.

Decisiones de la Fase 8, verificadas con medición:

- El desglose por núcleo se lee con `NtQuerySystemInformation` en lugar de PDH: evita la coma flotante y las rutas de contador localizadas, y cuesta 0-512 B por binario.
- `GetProcessTimes` y `K32GetProcessMemoryInfo` comparten un único `OpenProcess` por PID y muestra, en lugar de abrir dos handles: el tiempo de `processes` sube sólo por la segunda muestra (100 ms), no por llamadas extra.
- `temperature` dejó de devolver `Unsupported`: la lista vacía mantiene el exit 0 y evita que `status` dependa de sensores inexistentes.

Decisiones de la Fase 7, todas verificadas con medición:

- `opt-level = "z"` produce 199.168 B frente a 211.968 B de `"s"` en Windows x64: se mantiene `z`.
- En Linux x86_64, el enlazado estático es **menor** que el dinámico (388.824 B frente a 397.992 B) y no depende de glibc en el equipo destino: se mantiene `-C target-feature=+crt-static`.
- Envolver `stdout` en `BufWriter` añadió 2.048 B y no mejoró el tiempo de `processes` de forma medible (18,32 vs 18,43 ms de mediana): se descartó.
- En Linux, el nombre del proceso se toma del `comm` dentro de `/proc/[pid]/stat` en lugar de abrir `/proc/[pid]/comm` por PID y por muestra: elimina cientos de aperturas de archivo por consulta con un coste de 216-544 B.

Los binarios de Linux se generan por enlazado cruzado: el subsistema WSL de este entorno no tenía su imagen montada, así que sus tiempos de ejecución quedan pendientes de un equipo Linux. Los comandos de medición están documentados arriba.

---

## 7. Plan de desarrollo

| Fase | Contenido | Estado |
| --- | --- | --- |
| 1 | Estructura mínima, CLI y compilación optimizada | ✅ Completada |
| 2 | Monitorización de CPU y memoria | ✅ Completada |
| 3 | Discos, temperatura y red | ✅ Completada |
| 4 | Procesos y watchdog | ✅ Completada |
| 5 | Configuración, historial y alertas | ✅ Completada |
| 6 | Salida JSON y pruebas | ✅ Completada |
| 7 | Optimización extrema y benchmarking | ✅ Completada |
| 8 | Limitaciones de Windows: CPU por núcleo y por proceso | ✅ Completada |
| 9 | Interfaz TUI con gráficas (ratatui), panel por defecto | ✅ Completada |

No se avanza de fase hasta que la anterior funcione y tenga pruebas. La salida JSON se produce con `--json` y convive con la legible: comparten colectores y `main` sólo elige el formateador. La única limitación de plataforma que queda es la temperatura en Windows, por carecer de API ligera; se informa como «sin sensores disponibles» sin provocar un error.

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
