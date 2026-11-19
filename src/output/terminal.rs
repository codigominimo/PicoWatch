//! Salida legible para humanos.
//!
//! Todas las funciones escriben en un `Write` genérico para poder probarlas
//! con un `Vec<u8>` y para evitar bloquear la salida estándar más de una vez.

use std::io::{self, Write};

use crate::cli::Command;
use crate::collectors::cpu::{CpuReport, CpuUsage};
use crate::collectors::disk::DiskInfo;
use crate::collectors::memory::MemoryInfo;
use crate::collectors::network::Interface;
use crate::collectors::processes::Process;
use crate::collectors::temperature::Sensor;
use crate::core::{NAME, VERSION};
use crate::watchdog::{Event, Kind};

const HELP: &str = "\
PicoWatch - monitor de sistemas extremadamente ligero

Uso: picowatch [comando]

Comandos:
  status         Resumen general del sistema
  cpu            Uso de CPU
  memory         Uso de memoria
  disk           Uso de disco
  temperature    Sensores de temperatura
  network        Tráfico por interfaz de red
  processes      Procesos en ejecución
  watch          Vigilancia de procesos y servicios
  config         Configuración
  version        Versión del programa
  help           Muestra esta ayuda

Opciones:
  -h, --help     Muestra esta ayuda
  -V, --version  Muestra la versión

Vigilancia:
  watch [--interval MS] [--ticks N] nombre [nombre...]
    --interval MS  Periodo entre muestras (por defecto 1000 ms)
    --ticks N      Número de muestras; 0 = indefinido
";

/// Escribe la ayuda completa.
pub fn help<W: Write>(out: &mut W) -> io::Result<()> {
    out.write_all(HELP.as_bytes())
}

/// Escribe la versión en una sola línea.
pub fn version<W: Write>(out: &mut W) -> io::Result<()> {
    writeln!(out, "{NAME} {VERSION}")
}

/// Informa de que un comando está reservado para una fase posterior.
pub fn pending<W: Write>(out: &mut W, command: Command) -> io::Result<()> {
    writeln!(
        out,
        "{command}: comando reservado, disponible en la Fase {}",
        phase(command)
    )
}

fn phase(command: Command) -> u8 {
    if command == Command::Config {
        5
    } else {
        1
    }
}

/// Escribe el resumen de CPU y de cada núcleo.
pub fn cpu<W: Write>(out: &mut W, report: &CpuReport) -> io::Result<()> {
    write_usage(out, "CPU", report.usage)?;
    if report.core_count > 1 {
        writeln!(out, "Núcleos: {}", report.core_count)?;
        for (index, usage) in report.core_usages.iter().enumerate() {
            write!(out, "  cpu{index}: ")?;
            write_usage(out, "", *usage)?;
        }
    }
    Ok(())
}

/// Escribe memoria total, disponible y usada.
pub fn memory<W: Write>(out: &mut W, info: MemoryInfo) -> io::Result<()> {
    write!(out, "Memoria: ")?;
    write_mib(out, info.total_bytes)?;
    write!(out, " totales, ")?;
    write_mib(out, info.available_bytes)?;
    write!(out, " disponibles, ")?;
    write_mib(out, info.used_bytes())?;
    write!(out, " usadas (")?;
    write_percentage(out, info.used_basis_points())?;
    writeln!(out, ")")
}

/// Escribe espacio total, disponible y usado del volumen raíz.
pub fn disk<W: Write>(out: &mut W, info: DiskInfo) -> io::Result<()> {
    write!(out, "Disco ({}): ", info.label())?;
    write_gib(out, info.total_bytes)?;
    write!(out, " totales, ")?;
    write_gib(out, info.available_bytes)?;
    write!(out, " disponibles, ")?;
    write_gib(out, info.used_bytes())?;
    write!(out, " usadas (")?;
    write_percentage(out, info.used_basis_points())?;
    writeln!(out, ")")
}

/// Escribe la lista de sensores térmicos.
pub fn temperature<W: Write>(out: &mut W, sensors: &[Sensor]) -> io::Result<()> {
    if sensors.is_empty() {
        return writeln!(out, "Temperatura: sin sensores disponibles");
    }
    writeln!(out, "Temperatura:")?;
    for sensor in sensors {
        write!(out, "  {}: ", sensor.name)?;
        write_celsius(out, sensor.millicelsius)?;
        writeln!(out)?;
    }
    Ok(())
}

/// Escribe el tráfico acumulado de cada interfaz.
pub fn network<W: Write>(out: &mut W, interfaces: &[Interface]) -> io::Result<()> {
    if interfaces.is_empty() {
        return writeln!(out, "Red: sin interfaces disponibles");
    }
    writeln!(out, "Red:")?;
    for interface in interfaces {
        write!(out, "  {}: RX ", interface.name)?;
        write_mib(out, interface.rx_bytes)?;
        write!(out, ", TX ")?;
        write_mib(out, interface.tx_bytes)?;
        writeln!(out)?;
    }
    Ok(())
}

/// Escribe el listado de procesos ordenado por uso de CPU.
pub fn processes<W: Write>(out: &mut W, processes: &[Process]) -> io::Result<()> {
    writeln!(out, "Procesos: {}", processes.len())?;
    for process in processes {
        write!(out, "  {:>7}  {:<24} ", process.pid, process.name)?;
        write_mib(out, process.rss_bytes)?;
        write!(out, "  {} hilos  ", process.threads)?;
        write_percentage(out, process.cpu_basis_points)?;
        writeln!(out)?;
    }
    Ok(())
}

/// Anuncia el inicio de una sesión de vigilancia.
pub fn watch_start<W: Write>(out: &mut W, names: &[String], interval_ms: u64) -> io::Result<()> {
    let noun = if names.len() == 1 {
        "proceso"
    } else {
        "procesos"
    };
    writeln!(
        out,
        "Vigilando {} {} (cada {} ms): {}",
        names.len(),
        noun,
        interval_ms,
        names.join(", ")
    )
}

/// Escribe un evento de aparición o desaparición.
pub fn watch_event<W: Write>(out: &mut W, event: &Event) -> io::Result<()> {
    let (verb, sign) = match event.kind {
        Kind::Started => ("iniciado", '+'),
        Kind::Stopped => ("detenido", '-'),
    };
    writeln!(
        out,
        "[watch] {sign} {verb}: {} (pid {})",
        event.name, event.pid
    )
}

/// Resumen general: CPU, memoria, disco, temperatura y red.
///
/// `sensors` es `None` cuando la plataforma no ofrece sensores (Windows).
pub fn status<W: Write>(
    out: &mut W,
    report: &CpuReport,
    info: MemoryInfo,
    disk_info: DiskInfo,
    sensors: Option<&[Sensor]>,
    interfaces: &[Interface],
) -> io::Result<()> {
    writeln!(out, "PicoWatch - estado del sistema")?;
    cpu(out, report)?;
    memory(out, info)?;
    disk(out, disk_info)?;
    match sensors {
        Some(list) => match list.iter().max_by_key(|sensor| sensor.millicelsius) {
            Some(hottest) => {
                let noun = if list.len() == 1 {
                    "sensor"
                } else {
                    "sensores"
                };
                write!(out, "Temperatura: {} {}, máx. ", list.len(), noun)?;
                write_celsius(out, hottest.millicelsius)?;
                writeln!(out)?;
            }
            None => writeln!(out, "Temperatura: sin sensores disponibles")?,
        },
        None => writeln!(out, "Temperatura: no disponible en esta plataforma")?,
    }
    let noun = if interfaces.len() == 1 {
        "interfaz"
    } else {
        "interfaces"
    };
    writeln!(out, "Red: {} {}", interfaces.len(), noun)
}

fn write_usage<W: Write>(out: &mut W, label: &str, usage: CpuUsage) -> io::Result<()> {
    if !label.is_empty() {
        write!(out, "{label}: ")?;
    }
    write_percentage(out, usage.basis_points)?;
    writeln!(out)
}

fn write_percentage<W: Write>(out: &mut W, basis_points: u32) -> io::Result<()> {
    write!(out, "{}.{:02}%", basis_points / 100, basis_points % 100)
}

fn write_mib<W: Write>(out: &mut W, bytes: u64) -> io::Result<()> {
    const MIB: u64 = 1024 * 1024;
    write!(out, "{}.{} MiB", bytes / MIB, (bytes % MIB) * 10 / MIB)
}

fn write_gib<W: Write>(out: &mut W, bytes: u64) -> io::Result<()> {
    const GIB: u64 = 1024 * 1024 * 1024;
    write!(out, "{}.{} GiB", bytes / GIB, (bytes % GIB) * 10 / GIB)
}

fn write_celsius<W: Write>(out: &mut W, millicelsius: i64) -> io::Result<()> {
    write!(
        out,
        "{}.{} °C",
        millicelsius / 1000,
        (millicelsius % 1000).abs() / 100
    )
}

#[cfg(test)]
mod tests {
    use super::{
        cpu, disk, help, memory, network, pending, processes, status, temperature, version,
        watch_event, watch_start, Command,
    };
    use crate::collectors::cpu::{CpuReport, CpuUsage};
    use crate::collectors::disk::DiskInfo;
    use crate::collectors::memory::MemoryInfo;
    use crate::collectors::network::Interface;
    use crate::collectors::processes::Process;
    use crate::collectors::temperature::Sensor;
    use crate::watchdog::{Event, Kind};

    #[test]
    fn ayuda_incluye_comandos_principales() {
        let mut buf = Vec::new();
        help(&mut buf).unwrap();
        let texto = String::from_utf8(buf).unwrap();
        assert!(texto.starts_with("PicoWatch - monitor"));
        for cmd in [
            "status",
            "cpu",
            "memory",
            "disk",
            "temperature",
            "network",
            "processes",
            "watch",
            "config",
        ] {
            assert!(texto.contains(cmd), "falta {cmd} en la ayuda");
        }
    }

    #[test]
    fn version_tiene_formato_correcto() {
        let mut buf = Vec::new();
        version(&mut buf).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            format!("picowatch {}\n", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn pendiente_indica_fase() {
        let mut buf = Vec::new();
        pending(&mut buf, Command::Config).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "config: comando reservado, disponible en la Fase 5\n"
        );
    }

    #[test]
    fn salida_no_escribe_en_otro_buffer() {
        let mut version_buf = Vec::new();
        let mut help_buf = Vec::new();
        version(&mut version_buf).unwrap();
        help(&mut help_buf).unwrap();
        assert!(version_buf.len() < help_buf.len());
        assert!(!help_buf.starts_with(b"picowatch"));
    }

    #[test]
    fn cpu_muestra_total_y_nucleos() {
        let report = CpuReport {
            usage: CpuUsage { basis_points: 1234 },
            core_usages: vec![CpuUsage { basis_points: 5000 }, CpuUsage::default()],
            core_count: 2,
        };
        let mut buf = Vec::new();
        cpu(&mut buf, &report).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "CPU: 12.34%\nNúcleos: 2\n  cpu0: 50.00%\n  cpu1: 0.00%\n"
        );
    }

    #[test]
    fn cpu_con_un_nucleo_no_imprime_lista() {
        let report = CpuReport {
            usage: CpuUsage { basis_points: 100 },
            core_usages: vec![CpuUsage { basis_points: 100 }],
            core_count: 1,
        };
        let mut buf = Vec::new();
        cpu(&mut buf, &report).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "CPU: 1.00%\n");
    }

    #[test]
    fn memory_muestra_mib_y_porcentaje() {
        let info = MemoryInfo {
            total_bytes: 16_384 * 1024,
            available_bytes: 12_288 * 1024,
        };
        let mut buf = Vec::new();
        memory(&mut buf, info).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "Memoria: 16.0 MiB totales, 12.0 MiB disponibles, 4.0 MiB usadas (25.00%)\n"
        );
    }

    #[test]
    fn disk_muestra_gib_y_porcentaje() {
        let gib = 1024u64 * 1024 * 1024;
        let info = DiskInfo {
            total_bytes: 4 * gib,
            available_bytes: gib,
        };
        let mut buf = Vec::new();
        disk(&mut buf, info).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "Disco (/): 4.0 GiB totales, 1.0 GiB disponibles, 3.0 GiB usadas (75.00%)\n"
        );
    }

    #[test]
    fn temperatura_muestra_sensores() {
        let sensors = [
            Sensor {
                name: "thermal_zone0".into(),
                millicelsius: 45200,
            },
            Sensor {
                name: "thermal_zone1".into(),
                millicelsius: -1200,
            },
        ];
        let mut buf = Vec::new();
        temperature(&mut buf, &sensors).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "Temperatura:\n  thermal_zone0: 45.2 °C\n  thermal_zone1: -1.2 °C\n"
        );
    }

    #[test]
    fn red_muestra_trafico() {
        let interfaces = [Interface {
            name: "eth0".into(),
            rx_bytes: 2 * 1024 * 1024,
            tx_bytes: 1024 * 1024,
        }];
        let mut buf = Vec::new();
        network(&mut buf, &interfaces).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "Red:\n  eth0: RX 2.0 MiB, TX 1.0 MiB\n"
        );
    }

    #[test]
    fn procesos_muestra_pid_memoria_hilos_y_cpu() {
        let list = [Process {
            pid: 1234,
            name: "init".into(),
            threads: 4,
            rss_bytes: 1024 * 1024,
            cpu_basis_points: 1234,
        }];
        let mut buf = Vec::new();
        processes(&mut buf, &list).unwrap();
        let texto = String::from_utf8(buf).unwrap();
        assert!(texto.starts_with("Procesos: 1\n"));
        assert!(texto.contains("1234"));
        assert!(texto.contains("init"));
        assert!(texto.contains("1.0 MiB"));
        assert!(texto.contains("4 hilos"));
        assert!(texto.contains("12.34%\n"));
    }

    #[test]
    fn processes_vacio_no_imprime_filas() {
        let mut buf = Vec::new();
        processes(&mut buf, &[]).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "Procesos: 0\n");
    }

    #[test]
    fn vigilancia_muestra_inicio_y_eventos() {
        let names = vec!["init".to_string(), "sshd".to_string()];
        let mut buf = Vec::new();
        watch_start(&mut buf, &names, 250).unwrap();
        watch_event(
            &mut buf,
            &Event {
                kind: Kind::Started,
                name: "init".into(),
                pid: 1,
            },
        )
        .unwrap();
        watch_event(
            &mut buf,
            &Event {
                kind: Kind::Stopped,
                name: "sshd".into(),
                pid: 99,
            },
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "Vigilando 2 procesos (cada 250 ms): init, sshd\n\
             [watch] + iniciado: init (pid 1)\n\
             [watch] - detenido: sshd (pid 99)\n"
        );
    }

    #[test]
    fn status_incluye_todas_las_secciones() {
        let report = CpuReport {
            usage: CpuUsage::default(),
            core_usages: Vec::new(),
            core_count: 0,
        };
        let info = MemoryInfo {
            total_bytes: 1024 * 1024,
            available_bytes: 512 * 1024,
        };
        let gib = 1024u64 * 1024 * 1024;
        let disk_info = DiskInfo {
            total_bytes: 2 * gib,
            available_bytes: gib,
        };
        let sensors = [Sensor {
            name: "thermal_zone0".into(),
            millicelsius: 50000,
        }];
        let interfaces = [Interface {
            name: "lo".into(),
            rx_bytes: 0,
            tx_bytes: 0,
        }];

        let mut buf = Vec::new();
        status(
            &mut buf,
            &report,
            info,
            disk_info,
            Some(&sensors),
            &interfaces,
        )
        .unwrap();
        let texto = String::from_utf8(buf).unwrap();
        assert!(texto.starts_with("PicoWatch - estado del sistema\nCPU: 0.00%\n"));
        assert!(texto.contains("0.5 MiB usadas (50.00%)"));
        assert!(texto.contains("1.0 GiB usadas (50.00%)"));
        assert!(texto.contains("Temperatura: 1 sensor, máx. 50.0 °C"));
        assert!(texto.ends_with("Red: 1 interfaz\n"));
    }

    #[test]
    fn status_sin_sensores_en_windows() {
        let report = CpuReport::default();
        let info = MemoryInfo::default();
        let mut buf = Vec::new();
        status(&mut buf, &report, info, DiskInfo::default(), None, &[]).unwrap();
        assert!(String::from_utf8(buf)
            .unwrap()
            .contains("Temperatura: no disponible en esta plataforma"));
    }
}
