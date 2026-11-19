//! Uso de CPU a partir de `/proc/stat`.
//!
//! Los contadores son acumulativos en jiffies, así que el porcentaje se
//! calcula entre dos muestras. Se usa aritmética entera en centésimas de
//! porcentaje (`basis_points`, 0..=10000) para evitar `f64` y su coste de
//! formato en el ejecutable.

#[cfg(any(target_os = "linux", test))]
use std::fmt;
#[cfg(target_os = "linux")]
use std::fs;
use std::io;
use std::thread;
use std::time::Duration;

#[cfg(target_os = "linux")]
const PROC_STAT: &str = "/proc/stat";

/// Intervalo por defecto entre las dos muestras de una consulta puntual.
pub const DEFAULT_INTERVAL: Duration = Duration::from_millis(100);

/// Contadores acumulativos de una línea `cpu`.
///
/// `guest` y `guest_nice` se descartan: el kernel ya los incluye en `user` y
/// `nice`, y contarlos dos veces sesgaría el total.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

impl CpuTimes {
    /// Todos los estados que aportan tiempo de CPU.
    pub fn total(self) -> u64 {
        self.user
            .saturating_add(self.nice)
            .saturating_add(self.system)
            .saturating_add(self.idle)
            .saturating_add(self.iowait)
            .saturating_add(self.irq)
            .saturating_add(self.softirq)
            .saturating_add(self.steal)
    }

    /// `idle` más `iowait`: tiempo en el que la CPU no ejecutó trabajo.
    pub fn idle(self) -> u64 {
        self.idle.saturating_add(self.iowait)
    }
}

/// Una lectura instantánea de `/proc/stat`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CpuSnapshot {
    pub total: CpuTimes,
    pub cores: Vec<CpuTimes>,
}

impl CpuSnapshot {
    /// Porcentaje ocupado entre esta muestra y una anterior.
    ///
    /// Devuelve `None` si no avanzaron los contadores (no se puede dividir).
    pub fn usage_since(&self, previous: &Self) -> Option<CpuUsage> {
        let total = self.total.total().saturating_sub(previous.total.total());
        if total == 0 {
            return None;
        }
        let idle = self.total.idle().saturating_sub(previous.total.idle());
        Some(CpuUsage::from_ratio(total.saturating_sub(idle), total))
    }

    /// Porcentaje de cada núcleo presente en ambas muestras.
    pub fn core_usages_since(&self, previous: &Self) -> Vec<CpuUsage> {
        self.cores
            .iter()
            .zip(previous.cores.iter())
            .map(|(now, before)| {
                let total = now.total().saturating_sub(before.total());
                if total == 0 {
                    return CpuUsage::default();
                }
                let idle = now.idle().saturating_sub(before.idle());
                CpuUsage::from_ratio(total.saturating_sub(idle), total)
            })
            .collect()
    }
}

/// Porcentaje de uso expresado en centésimas (5000 = 50,00 %).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuUsage {
    pub basis_points: u32,
}

impl CpuUsage {
    fn from_ratio(part: u64, whole: u64) -> Self {
        let value = (part as u128 * 10_000 / whole as u128) as u32;
        Self {
            basis_points: value.min(10_000),
        }
    }
}

/// Resultado de una consulta de CPU.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CpuReport {
    pub usage: CpuUsage,
    pub core_usages: Vec<CpuUsage>,
    pub core_count: usize,
}

/// Error de parseo de `/proc/stat`.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    TooFewFields,
    InvalidField,
    MissingTotal,
}

#[cfg(any(target_os = "linux", test))]
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewFields => f.write_str("línea cpu con menos de 4 campos"),
            Self::InvalidField => f.write_str("campo de CPU no numérico"),
            Self::MissingTotal => f.write_str("falta la línea cpu en /proc/stat"),
        }
    }
}

/// Parsea el contenido de `/proc/stat`.
#[cfg(any(target_os = "linux", test))]
pub fn parse_snapshot(text: &str) -> Result<CpuSnapshot, ParseError> {
    let mut total = None;
    let mut cores = Vec::new();

    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let label = match fields.next() {
            Some(label) => label,
            None => continue,
        };

        // Sólo se parsean las líneas de CPU: `intr`, `ctxt`, etc. tienen
        // formatos distintos y deben ignorarse.
        match label.strip_prefix("cpu") {
            Some("") => total = Some(parse_times(fields)?),
            Some(rest) if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) => {
                cores.push(parse_times(fields)?);
            }
            _ => {}
        }
    }

    Ok(CpuSnapshot {
        total: total.ok_or(ParseError::MissingTotal)?,
        cores,
    })
}

#[cfg(any(target_os = "linux", test))]
fn parse_times<'a>(fields: impl Iterator<Item = &'a str>) -> Result<CpuTimes, ParseError> {
    let mut values = [0u64; 8];
    let mut count = 0;
    for (slot, field) in values.iter_mut().zip(fields) {
        *slot = field.parse().map_err(|_| ParseError::InvalidField)?;
        count += 1;
    }
    if count < 4 {
        return Err(ParseError::TooFewFields);
    }
    Ok(CpuTimes {
        user: values[0],
        nice: values[1],
        system: values[2],
        idle: values[3],
        iowait: values[4],
        irq: values[5],
        softirq: values[6],
        steal: values[7],
    })
}

/// Lee y parsea `/proc/stat`.
#[cfg(target_os = "linux")]
pub fn read() -> io::Result<CpuSnapshot> {
    let text = fs::read_to_string(PROC_STAT)?;
    parse_snapshot(&text)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

/// Obtiene los contadores agregados con `GetSystemTimes` y los de cada núcleo
/// con la clase 8 de `NtQuerySystemInformation` (no hay equivalente documentado
/// por núcleo). En ambos casos `kernel` incluye `idle` y se resta para no
/// contar dos veces el tiempo de inactividad.
#[cfg(target_os = "windows")]
pub fn read() -> io::Result<CpuSnapshot> {
    let (user, kernel, idle) = crate::platform::windows::cpu_times()?;
    let cores = crate::platform::windows::cpu_core_times()?
        .into_iter()
        .map(|(user, system, idle)| CpuTimes {
            user,
            system,
            idle,
            ..CpuTimes::default()
        })
        .collect();
    Ok(CpuSnapshot {
        total: CpuTimes {
            user,
            system: kernel.saturating_sub(idle),
            idle,
            ..CpuTimes::default()
        },
        cores,
    })
}

/// Toma dos muestras separadas por `interval` y calcula los porcentajes.
pub fn sample(interval: Duration) -> io::Result<CpuReport> {
    let first = read()?;
    thread::sleep(interval);
    let second = read()?;
    Ok(CpuReport {
        usage: second.usage_since(&first).unwrap_or_default(),
        core_usages: second.core_usages_since(&first),
        core_count: second.cores.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_snapshot, CpuSnapshot, CpuUsage, ParseError};

    const STAT: &str = "\
cpu  100 20 30 500 10 5 6 7 900 901
cpu0 50 10 15 250 5 2 3 4 450 451
cpu1 50 10 15 250 5 3 3 3 450 450
intr 12345 1 2 3
ctxt 987
btime 1
";

    #[test]
    fn parsea_totales_y_nucleos() {
        let snapshot = parse_snapshot(STAT).unwrap();
        assert_eq!(snapshot.total.user, 100);
        assert_eq!(snapshot.total.iowait, 10);
        assert_eq!(snapshot.total.steal, 7);
        assert_eq!(snapshot.cores.len(), 2);
        assert_eq!(snapshot.cores[0].user, 50);
        assert_eq!(snapshot.cores[1].irq, 3);
    }

    #[test]
    fn no_cuenta_guest_dos_veces() {
        let snapshot = parse_snapshot(STAT).unwrap();
        // 100+20+30+500+10+5+6+7 = 678; los campos 900/901 se ignoran.
        assert_eq!(snapshot.total.total(), 678);
        assert_eq!(snapshot.total.idle(), 510);
    }

    #[test]
    fn calcula_porcentaje_entre_muestras() {
        let first = parse_snapshot("cpu 100 0 0 100 0 0 0 0\ncpu0 100 0 0 100 0 0 0 0\n").unwrap();
        let second = parse_snapshot("cpu 150 0 0 125 0 0 0 0\ncpu0 150 0 0 125 0 0 0 0\n").unwrap();
        assert_eq!(
            second.usage_since(&first),
            Some(CpuUsage { basis_points: 6666 })
        );
        assert_eq!(
            second.core_usages_since(&first),
            vec![CpuUsage { basis_points: 6666 }]
        );
    }

    #[test]
    fn muestra_sin_avance_no_da_porcentaje() {
        let snapshot = parse_snapshot("cpu 10 0 0 10 0 0 0 0\n").unwrap();
        assert_eq!(snapshot.usage_since(&snapshot), None);
    }

    #[test]
    fn porcentaje_nunca_supera_cien() {
        let first = CpuSnapshot::default();
        let second = CpuSnapshot {
            total: super::CpuTimes {
                user: 100,
                ..Default::default()
            },
            cores: Vec::new(),
        };
        assert_eq!(second.usage_since(&first).unwrap().basis_points, 10_000);
    }

    #[test]
    fn rechaza_lineas_cpu_mal_formadas() {
        assert_eq!(parse_snapshot("cpu 1 2 3\n"), Err(ParseError::TooFewFields));
        assert_eq!(
            parse_snapshot("cpu 1 2 3 x\n"),
            Err(ParseError::InvalidField)
        );
        assert_eq!(
            parse_snapshot("intr 1 2 3 4\n"),
            Err(ParseError::MissingTotal)
        );
    }

    #[test]
    fn porcentaje_se_trunca_a_dos_decimales() {
        let first = parse_snapshot("cpu 0 0 0 0 0 0 0 0\n").unwrap();
        let second = parse_snapshot("cpu 3 0 0 0 0 0 0 0\n").unwrap();
        assert_eq!(second.usage_since(&first).unwrap().basis_points, 10_000);
        let half = parse_snapshot("cpu 1 0 0 2 0 0 0 0\n").unwrap();
        assert_eq!(half.usage_since(&first).unwrap().basis_points, 3333);
    }
}
