//! Listado de procesos con PID, nombre, hilos, RSS y uso de CPU.
//!
//! Linux: `/proc/[pid]/stat`, dos muestras separadas para calcular CPU.
//! Windows: Toolhelp32, `K32GetProcessMemoryInfo` y `GetProcessTimes`.

use std::io;
use std::thread;
use std::time::Duration;

/// Intervalo por defecto entre las dos muestras de CPU.
pub const DEFAULT_INTERVAL: Duration = Duration::from_millis(100);

/// Convierte un incremento de contador en puntos base (10000 = un núcleo lleno).
///
/// `capacity` es el avance que tendría un núcleo ocupado al 100 % durante el
/// intervalo, expresado en las mismas unidades que `busy`: ticks de reloj en
/// Linux y unidades de 100 ns en Windows. Es pura y compartida por ambos.
fn usage_basis_points(busy: u64, capacity: u64) -> u32 {
    busy.saturating_mul(10_000)
        .checked_div(capacity)
        .unwrap_or(0)
        .min(10_000) as u32
}

/// Proceso ya normalizado para salida.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub threads: u32,
    pub rss_bytes: u64,
    pub cpu_basis_points: u32,
}

/// Datos crudos de una única lectura de `/proc/[pid]/stat`.
///
/// `name` se extrae del propio `stat` (el `comm` entre paréntesis) para no
/// abrir `/proc/[pid]/comm` por cada proceso en cada muestra.
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawProcess {
    pub pid: u32,
    pub name: String,
    pub ticks: u64,
    pub rss_pages: u64,
    pub threads: u32,
}

/// Error de parseo de `/proc/[pid]/stat`.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    MissingComm,
    TooShort,
    InvalidField,
}

#[cfg(any(target_os = "linux", test))]
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingComm => f.write_str("falta el nombre entre paréntesis"),
            Self::TooShort => f.write_str("línea stat demasiado corta"),
            Self::InvalidField => f.write_str("campo stat no numérico"),
        }
    }
}

/// Parsea una línea de `/proc/[pid]/stat`.
///
/// `comm` puede contener espacios y paréntesis, así que se delimita por el
/// primer `(` y el último `)`. Tras el `)` los índices son: state 0, utime 11,
/// stime 12, num_threads 17 y rss 21 (campo 24 del manual de `proc_stat`).
#[cfg(any(target_os = "linux", test))]
pub fn parse_stat(text: &str) -> Result<RawProcess, ParseError> {
    let open = text.find('(').ok_or(ParseError::MissingComm)?;
    let close = text.rfind(')').ok_or(ParseError::MissingComm)?;
    let pid = text[..open]
        .trim()
        .parse::<u32>()
        .map_err(|_| ParseError::InvalidField)?;
    let name = text[open + 1..close].to_owned();

    let mut fields = text[close + 1..].split_whitespace();
    fields.next().ok_or(ParseError::TooShort)?; // state
    let utime = field(&mut fields, 10)?;
    let stime = fields.next().ok_or(ParseError::TooShort)?;
    let stime = stime.parse::<u64>().map_err(|_| ParseError::InvalidField)?;
    let threads = field(&mut fields, 4)?;
    let rss_pages = field(&mut fields, 3)?;

    Ok(RawProcess {
        pid,
        name,
        ticks: utime.saturating_add(stime),
        rss_pages,
        threads: threads as u32,
    })
}

#[cfg(any(target_os = "linux", test))]
fn field<'a>(fields: &mut impl Iterator<Item = &'a str>, skip: usize) -> Result<u64, ParseError> {
    fields
        .nth(skip)
        .ok_or(ParseError::TooShort)?
        .parse::<u64>()
        .map_err(|_| ParseError::InvalidField)
}

/// Toma una única muestra de todos los procesos (sin uso de CPU).
#[cfg(target_os = "linux")]
pub fn list() -> io::Result<Vec<Process>> {
    let page = page_size();
    Ok(raw_list()?
        .into_iter()
        .map(|raw| Process {
            pid: raw.pid,
            name: raw.name,
            threads: raw.threads,
            rss_bytes: raw.rss_pages.saturating_mul(page),
            cpu_basis_points: 0,
        })
        .collect())
}

/// Toma dos muestras y calcula el uso de CPU y la memoria de cada proceso.
///
/// El porcentaje es relativo a un núcleo, igual que en `top`; está acotado
/// a 10000 puntos base. Los procesos que desaparecen entre muestras se omiten.
#[cfg(target_os = "linux")]
pub fn sample(interval: Duration) -> io::Result<Vec<Process>> {
    let first = raw_list()?;
    thread::sleep(interval);
    let second = raw_list()?;

    let period_ticks = clock_ticks().saturating_mul(interval.as_millis().max(1) as u64) / 1000;
    let page = page_size();

    let mut processes = Vec::with_capacity(second.len());
    for current in second {
        let previous_ticks = first
            .iter()
            .find(|old| old.pid == current.pid)
            .map_or(current.ticks, |old| old.ticks);
        let busy = current.ticks.saturating_sub(previous_ticks);
        let cpu = usage_basis_points(busy, period_ticks);
        processes.push(Process {
            pid: current.pid,
            name: current.name,
            threads: current.threads,
            rss_bytes: current.rss_pages.saturating_mul(page),
            cpu_basis_points: cpu,
        });
    }
    processes.sort_unstable_by(|a, b| {
        b.cpu_basis_points
            .cmp(&a.cpu_basis_points)
            .then_with(|| a.pid.cmp(&b.pid))
    });
    Ok(processes)
}

#[cfg(target_os = "linux")]
fn raw_list() -> io::Result<Vec<RawProcess>> {
    use std::fs;

    let mut result = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let pid = match entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        {
            Some(pid) => pid,
            None => continue,
        };
        let text = match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(text) => text,
            Err(_) => continue, // el proceso pudo terminar mientras se enumeraba
        };
        if let Ok(raw) = parse_stat(&text) {
            result.push(raw);
        }
    }
    Ok(result)
}

#[cfg(target_os = "linux")]
extern "C" {
    fn sysconf(name: std::os::raw::c_int) -> std::os::raw::c_long;
}

#[cfg(target_os = "linux")]
fn page_size() -> u64 {
    // SAFETY: `sysconf` no recibe punteros y devuelve un valor o -1.
    let value = unsafe { sysconf(30) }; // _SC_PAGESIZE
    if value > 0 {
        value as u64
    } else {
        4096
    }
}

#[cfg(target_os = "linux")]
fn clock_ticks() -> u64 {
    // SAFETY: igual que `page_size`.
    let value = unsafe { sysconf(2) }; // _SC_CLK_TCK
    if value > 0 {
        value as u64
    } else {
        100
    }
}

#[cfg(target_os = "windows")]
pub fn list() -> io::Result<Vec<Process>> {
    crate::platform::windows::processes()
}

/// Toma dos muestras y calcula el uso de CPU de cada proceso.
///
/// `GetProcessTimes` devuelve el tiempo acumulado en unidades de 100 ns, así
/// que un núcleo al 100 % avanza `interval_ms * 10_000` unidades. El
/// porcentaje es relativo a un núcleo, igual que en `top` y que en Linux.
#[cfg(target_os = "windows")]
pub fn sample(interval: Duration) -> io::Result<Vec<Process>> {
    let first = crate::platform::windows::process_samples()?;
    thread::sleep(interval);
    let second = crate::platform::windows::process_samples()?;

    // Un milisegundo tiene 10_000 unidades de 100 ns.
    let capacity = interval.as_millis().max(1) as u64 * 10_000;
    let mut processes = Vec::with_capacity(second.len());
    for current in second {
        let previous = first
            .iter()
            .find(|old| old.pid == current.pid)
            .map_or(current.cpu_100ns, |old| old.cpu_100ns);
        let busy = current.cpu_100ns.saturating_sub(previous);
        processes.push(Process {
            pid: current.pid,
            name: current.name,
            threads: current.threads,
            rss_bytes: current.rss_bytes,
            cpu_basis_points: usage_basis_points(busy, capacity),
        });
    }
    processes.sort_unstable_by(|a, b| {
        b.cpu_basis_points
            .cmp(&a.cpu_basis_points)
            .then_with(|| a.pid.cmp(&b.pid))
    });
    Ok(processes)
}

#[cfg(test)]
mod tests {
    use super::{parse_stat, usage_basis_points, ParseError};

    const STAT: &str = "1234 (mi proceso) R 1 1 1 0 -1 4194304 100 0 0 0 250 50 0 0 20 0 4 0 99 1000 50000 250 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n";

    #[test]
    fn parsea_stat_con_espacios_en_el_nombre() {
        let raw = parse_stat(STAT).unwrap();
        assert_eq!(raw.pid, 1234);
        assert_eq!(raw.name, "mi proceso");
        assert_eq!(raw.ticks, 300);
        assert_eq!(raw.threads, 4);
        assert_eq!(raw.rss_pages, 50_000);
    }

    #[test]
    fn acepta_parentesis_en_el_nombre() {
        let line = STAT.replace("(mi proceso)", "(mi (proceso) raro)");
        let raw = parse_stat(&line).unwrap();
        assert_eq!(raw.name, "mi (proceso) raro");
        assert_eq!(raw.ticks, 300);
    }

    #[test]
    fn calcula_cpu_relativa_a_un_nucleo() {
        // 5000 unidades de 100 ns en 100 ms = medio núcleo (5000 puntos base).
        assert_eq!(usage_basis_points(500_000, 1_000_000), 5000);
        // Un núcleo completo nunca supera el 100 %.
        assert_eq!(usage_basis_points(2_000_000, 1_000_000), 10_000);
        assert_eq!(usage_basis_points(0, 1_000_000), 0);
        assert_eq!(usage_basis_points(500_000, 0), 0);
    }

    #[test]
    fn rechaza_lineas_invalidas() {
        assert_eq!(
            parse_stat("1234 sin parentesis R 1"),
            Err(ParseError::MissingComm)
        );
        assert_eq!(parse_stat("1234 (x) R"), Err(ParseError::TooShort));
        assert_eq!(
            parse_stat("abc (x) R 1 1 1 0 0 0 0 0 0 0 1 1"),
            Err(ParseError::InvalidField)
        );
    }
}
