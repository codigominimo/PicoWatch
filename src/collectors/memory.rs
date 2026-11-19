//! Memoria RAM a partir de `/proc/meminfo`.
//!
//! Se usan `MemTotal` y `MemAvailable`. `MemAvailable` ya descuenta la memoria
//! reclaimable (cachés y buffers), por lo que `usada = total - disponible` es
//! la cifra que mejor refleja el consumo real.

#[cfg(any(target_os = "linux", test))]
use std::fmt;
#[cfg(target_os = "linux")]
use std::fs;
use std::io;

#[cfg(target_os = "linux")]
const PROC_MEMINFO: &str = "/proc/meminfo";
#[cfg(any(target_os = "linux", test))]
const KIB: u64 = 1024;

/// Memoria expresada en bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

impl MemoryInfo {
    /// RAM no disponible para nuevas asignaciones.
    pub fn used_bytes(self) -> u64 {
        self.total_bytes.saturating_sub(self.available_bytes)
    }

    /// Porcentaje usado en centésimas (0..=10000).
    pub fn used_basis_points(self) -> u32 {
        if self.total_bytes == 0 {
            return 0;
        }
        let used = self.used_bytes() as u128 * 10_000 / self.total_bytes as u128;
        used.min(10_000) as u32
    }
}

/// Error de parseo de `/proc/meminfo`.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    Missing(&'static str),
    InvalidValue,
    InvalidUnit,
}

#[cfg(any(target_os = "linux", test))]
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(field) => write!(f, "falta {field} en /proc/meminfo"),
            Self::InvalidValue => f.write_str("valor de memoria no numérico"),
            Self::InvalidUnit => f.write_str("unidad de memoria desconocida (se esperaba kB)"),
        }
    }
}

/// Parsea el contenido de `/proc/meminfo`.
#[cfg(any(target_os = "linux", test))]
pub fn parse_meminfo(text: &str) -> Result<MemoryInfo, ParseError> {
    let mut total = None;
    let mut available = None;

    for line in text.lines() {
        let (label, value) = match line.split_once(':') {
            Some(parts) => parts,
            None => continue,
        };
        let destination = match label {
            "MemTotal" => &mut total,
            "MemAvailable" => &mut available,
            _ => continue,
        };
        let mut fields = value.split_whitespace();
        let amount = fields.next().ok_or(ParseError::InvalidValue)?;
        let unit = fields.next().ok_or(ParseError::InvalidUnit)?;
        if unit != "kB" {
            return Err(ParseError::InvalidUnit);
        }
        let kib = amount
            .parse::<u64>()
            .map_err(|_| ParseError::InvalidValue)?;
        *destination = Some(kib.saturating_mul(KIB));
    }

    Ok(MemoryInfo {
        total_bytes: total.ok_or(ParseError::Missing("MemTotal"))?,
        available_bytes: available.ok_or(ParseError::Missing("MemAvailable"))?,
    })
}

/// Lee y parsea `/proc/meminfo`.
#[cfg(target_os = "linux")]
pub fn read() -> io::Result<MemoryInfo> {
    let text = fs::read_to_string(PROC_MEMINFO)?;
    parse_meminfo(&text)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

/// Obtiene la memoria física mediante `GlobalMemoryStatusEx`.
#[cfg(target_os = "windows")]
pub fn read() -> io::Result<MemoryInfo> {
    let (total, available) = crate::platform::windows::memory_bytes()?;
    Ok(MemoryInfo {
        total_bytes: total,
        available_bytes: available,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_meminfo, MemoryInfo, ParseError};

    const MEMINFO: &str = "\
MemTotal:       16384 kB
MemFree:         8192 kB
MemAvailable:   12288 kB
Buffers:          100 kB
Cached:          2000 kB
SwapTotal:           0 kB
HugePages_Total:     0
";

    #[test]
    fn parsea_y_convierte_a_bytes() {
        let info = parse_meminfo(MEMINFO).unwrap();
        assert_eq!(info.total_bytes, 16_384 * 1024);
        assert_eq!(info.available_bytes, 12_288 * 1024);
        assert_eq!(info.used_bytes(), 4_096 * 1024);
        assert_eq!(info.used_basis_points(), 2500);
    }

    #[test]
    fn ignora_campos_no_solicitados() {
        let info = parse_meminfo("MemTotal: 1 kB\nMemAvailable: 1 kB\nMemFree: 999 kB\n").unwrap();
        assert_eq!(info.total_bytes, 1024);
    }

    #[test]
    fn detecta_campos_faltantes() {
        assert_eq!(
            parse_meminfo("MemTotal: 1 kB\n"),
            Err(ParseError::Missing("MemAvailable"))
        );
        assert_eq!(
            parse_meminfo("MemAvailable: 1 kB\n"),
            Err(ParseError::Missing("MemTotal"))
        );
    }

    #[test]
    fn detecta_valores_y_unidades_invalidas() {
        assert_eq!(
            parse_meminfo("MemTotal: x kB\nMemAvailable: 1 kB\n"),
            Err(ParseError::InvalidValue)
        );
        assert_eq!(
            parse_meminfo("MemTotal: 1 MB\nMemAvailable: 1 kB\n"),
            Err(ParseError::InvalidUnit)
        );
    }

    #[test]
    fn memoria_cero_no_divide_por_cero() {
        assert_eq!(MemoryInfo::default().used_basis_points(), 0);
    }
}
