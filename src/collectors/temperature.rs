//! Sensores térmicos desde `/sys/class/thermal`.
//!
//! Cada `thermal_zoneN/temp` contiene miligrados Celsius. Windows no expone
//! una API ligera equivalente: la única fuente es WMI/COM, que añadiría cientos
//! de líneas y dependencias de runtime. En Windows se devuelve una lista vacía
//! para que `temperature` y `status` sigan funcionando sin error de E/S.

use std::io;

/// Sensor con su lectura en miligrados Celsius.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sensor {
    pub name: String,
    pub millicelsius: i64,
}

/// Convierte el contenido de un archivo `temp` del kernel.
#[cfg(any(target_os = "linux", test))]
pub fn parse_temperature(text: &str) -> Result<i64, ParseError> {
    text.trim()
        .parse::<i64>()
        .map_err(|_| ParseError::InvalidValue)
}

/// Error de parseo de un sensor.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    InvalidValue,
}

#[cfg(any(target_os = "linux", test))]
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("valor de temperatura no numérico")
    }
}

/// Lista los sensores disponibles.
#[cfg(target_os = "linux")]
pub fn read() -> io::Result<Vec<Sensor>> {
    use std::fs;
    use std::path::PathBuf;

    const BASE: &str = "/sys/class/thermal";

    let mut zones: Vec<PathBuf> = fs::read_dir(BASE)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map_or(false, |name| name.starts_with("thermal_zone"))
        })
        .collect();
    zones.sort();

    let mut sensors = Vec::with_capacity(zones.len());
    for zone in zones {
        let name = match zone.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_owned(),
            None => continue,
        };
        // Una zona puede estar desconectada: se ignora sin fallar el resto.
        let raw = match fs::read_to_string(zone.join("temp")) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        if let Ok(millicelsius) = parse_temperature(&raw) {
            sensors.push(Sensor { name, millicelsius });
        }
    }
    Ok(sensors)
}

/// Windows no ofrece sensores térmicos sin recurrir a WMI/COM, así que se
/// informa de "sin sensores" en lugar de fallar con `Unsupported`.
#[cfg(target_os = "windows")]
pub fn read() -> io::Result<Vec<Sensor>> {
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::{parse_temperature, ParseError, Sensor};

    #[test]
    fn parsea_miligrados() {
        assert_eq!(parse_temperature("45200\n"), Ok(45200));
        assert_eq!(parse_temperature("-1500\n"), Ok(-1500));
        assert_eq!(parse_temperature("abc\n"), Err(ParseError::InvalidValue));
    }

    #[test]
    fn sensor_es_comparable() {
        let sensor = Sensor {
            name: "thermal_zone0".into(),
            millicelsius: 45200,
        };
        assert_eq!(sensor.name, "thermal_zone0");
        assert_eq!(sensor.millicelsius, 45200);
    }
}
