//! Historial de muestras en un archivo de texto delimitado por tabuladores.
//!
//! Se elige TSV sobre JSON para no añadir dependencias y mantener el tamaño:
//! una línea es legible con `cat`/`type` y se parsea con `split`. El archivo se
//! reescribe al añadir para poder aplicar el límite de entradas; con 200
//! líneas el coste es trivial.

use std::fs;
use std::io;
use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

/// Una muestra del estado del sistema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    /// Segundos desde el epoch Unix.
    pub timestamp: u64,
    pub cpu_basis_points: u32,
    pub memory_basis_points: u32,
    pub disk_basis_points: u32,
    /// `None` en plataformas sin sensores térmicos.
    pub temperature_millicelsius: Option<i64>,
    pub interfaces: u32,
}

/// Error de parseo de una línea del historial.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    TooShort,
    InvalidValue,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => f.write_str("línea de historial incompleta"),
            Self::InvalidValue => f.write_str("valor de historial no numérico"),
        }
    }
}

fn parse<T: FromStr>(field: &str) -> Result<T, ParseError> {
    field.parse::<T>().map_err(|_| ParseError::InvalidValue)
}

/// Serializa una muestra a una línea terminada en salto de línea.
pub fn format_line(record: &Record) -> String {
    let temperature = match record.temperature_millicelsius {
        Some(value) => value.to_string(),
        None => "-".to_owned(),
    };
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\n",
        record.timestamp,
        record.cpu_basis_points,
        record.memory_basis_points,
        record.disk_basis_points,
        temperature,
        record.interfaces
    )
}

/// Parsea una línea del historial.
pub fn parse_line(line: &str) -> Result<Record, ParseError> {
    let fields: Vec<&str> = line.trim_end_matches(['\n', '\r']).split('\t').collect();
    if fields.len() != 6 {
        return Err(ParseError::TooShort);
    }
    let temperature = if fields[4] == "-" {
        None
    } else {
        Some(parse::<i64>(fields[4])?)
    };
    Ok(Record {
        timestamp: parse(fields[0])?,
        cpu_basis_points: parse(fields[1])?,
        memory_basis_points: parse(fields[2])?,
        disk_basis_points: parse(fields[3])?,
        temperature_millicelsius: temperature,
        interfaces: parse(fields[5])?,
    })
}

/// Marca de tiempo actual; `0` si el reloj del sistema está antes de 1970.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_all(path: &Path) -> io::Result<Vec<Record>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    // Una línea corrupta se ignora: no debe impedir tomar nuevas muestras.
    Ok(text
        .lines()
        .filter_map(|line| parse_line(line).ok())
        .collect())
}

fn write_all(path: &Path, records: &[Record]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut text = String::new();
    for record in records {
        text.push_str(&format_line(record));
    }
    fs::write(path, text)
}

/// Añade una muestra y conserva como máximo `max` entradas (`0` = sin límite).
pub fn append(path: &Path, record: Record, max: usize) -> io::Result<()> {
    let mut records = read_all(path)?;
    records.push(record);
    if max > 0 && records.len() > max {
        let excess = records.len() - max;
        records.drain(..excess);
    }
    write_all(path, &records)
}

/// Devuelve las últimas `count` muestras, de antigua a reciente.
pub fn read_last(path: &Path, count: usize) -> io::Result<Vec<Record>> {
    let records = read_all(path)?;
    let start = records.len().saturating_sub(count);
    Ok(records[start..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::{append, format_line, parse_line, read_last, ParseError, Record};
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    fn sample() -> Record {
        Record {
            timestamp: 1_700_000_000,
            cpu_basis_points: 1234,
            memory_basis_points: 5000,
            disk_basis_points: 7500,
            temperature_millicelsius: Some(45_200),
            interfaces: 3,
        }
    }

    #[test]
    fn linea_va_y_vuelve() {
        let record = sample();
        assert_eq!(parse_line(&format_line(&record)), Ok(record));
    }

    #[test]
    fn temperatura_ausente_usa_guion() {
        let mut record = sample();
        record.temperature_millicelsius = None;
        let line = format_line(&record);
        assert!(line.contains("\t-\t"));
        assert_eq!(parse_line(&line), Ok(record));
    }

    #[test]
    fn rechaza_lineas_mal_formadas() {
        assert_eq!(parse_line("1\t2\t3"), Err(ParseError::TooShort));
        assert_eq!(
            parse_line("1\tx\t3\t4\t5\t6"),
            Err(ParseError::InvalidValue)
        );
    }

    #[test]
    fn append_respeta_el_limite_y_lee_las_ultimas() {
        let path: PathBuf = env::temp_dir().join(format!(
            "picowatch-history-{}-{}.log",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_file(&path);
        for second in 1..=4 {
            let mut record = sample();
            record.timestamp = second;
            append(&path, record, 2).unwrap();
        }
        let last = read_last(&path, 2).unwrap();
        assert_eq!(
            last.iter()
                .map(|record| record.timestamp)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
        let _ = fs::remove_file(&path);
    }
}
