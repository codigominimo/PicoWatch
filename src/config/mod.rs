//! Configuración en texto plano, sin crates de serialización.
//!
//! Formato `clave=valor`, con comentarios `#`. La ruta por defecto sigue las
//! convenciones de cada sistema: `APPDATA` en Windows y `XDG_CONFIG_HOME` o
//! `~/.config` en Linux. Si no hay directorio conocido se usa el directorio
//! actual, para que el programa siga funcionando en entornos mínimos.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Umbrales y opciones persistentes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// CPU usada que dispara una alerta, en puntos base.
    pub cpu_alert: u32,
    /// Memoria usada que dispara una alerta, en puntos base.
    pub memory_alert: u32,
    /// Disco usado que dispara una alerta, en puntos base.
    pub disk_alert: u32,
    /// Temperatura máxima que dispara una alerta, en miligrados Celsius.
    pub temperature_alert: i64,
    /// Si `status` escribe una muestra en el historial.
    pub history_enabled: bool,
    /// Número máximo de líneas conservadas; `0` no limita.
    pub history_max: usize,
    /// Ruta explícita del historial; `None` usa la junto al archivo de config.
    pub history_path: Option<PathBuf>,
    /// Periodo por defecto de `watch`, en milisegundos.
    pub watch_interval_ms: u64,
    /// Muestras por defecto de `watch`; `0` es indefinido.
    pub watch_ticks: u32,
    /// Nombres vigilados por defecto.
    pub watch_names: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cpu_alert: 9000,
            memory_alert: 9000,
            disk_alert: 9000,
            temperature_alert: 80_000,
            history_enabled: true,
            history_max: 200,
            history_path: None,
            watch_interval_ms: 1000,
            watch_ticks: 0,
            watch_names: Vec::new(),
        }
    }
}

/// Error de sintaxis del archivo de configuración.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    InvalidLine(usize),
    InvalidValue(usize, String),
    UnknownKey(usize, String),
    OutOfRange(usize),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLine(line) => write!(f, "línea {line}: se esperaba clave=valor"),
            Self::InvalidValue(line, value) => {
                write!(f, "línea {line}: valor no válido: {value}")
            }
            Self::UnknownKey(line, key) => write!(f, "línea {line}: clave desconocida: {key}"),
            Self::OutOfRange(line) => {
                write!(f, "línea {line}: el porcentaje debe estar entre 0 y 10000")
            }
        }
    }
}

fn number<T: FromStr>(value: &str, line: usize) -> Result<T, ParseError> {
    value
        .parse::<T>()
        .map_err(|_| ParseError::InvalidValue(line, value.to_owned()))
}

fn basis_points(value: &str, line: usize) -> Result<u32, ParseError> {
    let parsed = number::<u32>(value, line)?;
    if parsed > 10_000 {
        return Err(ParseError::OutOfRange(line));
    }
    Ok(parsed)
}

fn boolean(value: &str, line: usize) -> Result<bool, ParseError> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(ParseError::InvalidValue(line, value.to_owned())),
    }
}

/// Parsea el contenido de un archivo de configuración.
pub fn parse(text: &str) -> Result<Config, ParseError> {
    let mut config = Config::default();
    for (index, raw) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or(ParseError::InvalidLine(line_number))?;
        let key = key.trim();
        let value = value.trim();
        match key {
            "cpu_alert" => config.cpu_alert = basis_points(value, line_number)?,
            "memory_alert" => config.memory_alert = basis_points(value, line_number)?,
            "disk_alert" => config.disk_alert = basis_points(value, line_number)?,
            "temperature_alert" => config.temperature_alert = number(value, line_number)?,
            "history_enabled" => config.history_enabled = boolean(value, line_number)?,
            "history_max" => config.history_max = number(value, line_number)?,
            "history_path" => {
                config.history_path = if value.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(value))
                };
            }
            "watch_interval_ms" => {
                config.watch_interval_ms = number(value, line_number)?;
                if config.watch_interval_ms == 0 {
                    return Err(ParseError::InvalidValue(line_number, value.to_owned()));
                }
            }
            "watch_ticks" => config.watch_ticks = number(value, line_number)?,
            "watch_names" => {
                config.watch_names = value
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            _ => return Err(ParseError::UnknownKey(line_number, key.to_owned())),
        }
    }
    Ok(config)
}

/// Serializa la configuración para `config init` o `config`.
pub fn to_text(config: &Config) -> String {
    let mut lines = vec![
        "# Configuración de PicoWatch".to_string(),
        "# Los porcentajes van en puntos base (0..10000).".to_string(),
        format!("cpu_alert={}", config.cpu_alert),
        format!("memory_alert={}", config.memory_alert),
        format!("disk_alert={}", config.disk_alert),
        format!("temperature_alert={}", config.temperature_alert),
        format!("history_enabled={}", config.history_enabled),
        format!("history_max={}", config.history_max),
        format!("watch_interval_ms={}", config.watch_interval_ms),
        format!("watch_ticks={}", config.watch_ticks),
    ];
    if let Some(path) = &config.history_path {
        lines.push(format!("history_path={}", path.display()));
    }
    lines.push(format!("watch_names={}", config.watch_names.join(",")));
    lines.push(String::new());
    lines.join("\n")
}

/// Ruta del archivo de configuración.
pub fn config_path() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(dir) = env::var_os("APPDATA") {
            if !dir.is_empty() {
                return PathBuf::from(dir).join("PicoWatch").join("config.txt");
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(dir) = env::var_os("XDG_CONFIG_HOME") {
            if !dir.is_empty() {
                return PathBuf::from(dir).join("picowatch").join("config");
            }
        }
        if let Some(home) = env::var_os("HOME") {
            if !home.is_empty() {
                return PathBuf::from(home)
                    .join(".config")
                    .join("picowatch")
                    .join("config");
            }
        }
    }
    PathBuf::from("picowatch.conf")
}

/// Carga la configuración; si no existe el archivo devuelve los valores por defecto.
pub fn load() -> io::Result<Config> {
    let text = match fs::read_to_string(config_path()) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(error) => return Err(error),
    };
    parse(&text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

/// Crea el archivo de configuración con valores por defecto si no existe.
pub fn init() -> io::Result<PathBuf> {
    let path = config_path();
    if path.exists() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(&path, to_text(&Config::default()))?;
    Ok(path)
}

/// Ruta del historial, explícita o junto al archivo de configuración.
pub fn history_file(config: &Config) -> PathBuf {
    match &config.history_path {
        Some(path) => path.clone(),
        None => config_path()
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("history.log"),
    }
}

#[cfg(test)]
mod tests {
    use super::{config_path, history_file, parse, to_text, Config, ParseError};
    use std::path::PathBuf;

    #[test]
    fn defaults_son_razonables() {
        let config = Config::default();
        assert_eq!(config.cpu_alert, 9000);
        assert_eq!(config.temperature_alert, 80_000);
        assert!(config.history_enabled);
        assert_eq!(config.history_max, 200);
    }

    #[test]
    fn parsea_configuracion_completa() {
        let text = "\
# comentario
cpu_alert=8500
memory_alert=8000
disk_alert=7500
temperature_alert=70000
history_enabled=false
history_max=25
history_path=/tmp/pw.log
watch_interval_ms=250
watch_ticks=3
watch_names=init, sshd ,
";
        let config = parse(text).unwrap();
        assert_eq!(config.cpu_alert, 8500);
        assert_eq!(config.disk_alert, 7500);
        assert_eq!(config.temperature_alert, 70_000);
        assert!(!config.history_enabled);
        assert_eq!(config.history_max, 25);
        assert_eq!(config.history_path, Some(PathBuf::from("/tmp/pw.log")));
        assert_eq!(config.watch_interval_ms, 250);
        assert_eq!(config.watch_ticks, 3);
        assert_eq!(config.watch_names, vec!["init", "sshd"]);
    }

    #[test]
    fn rechaza_entradas_invalidas() {
        assert_eq!(parse("sin igual").unwrap_err(), ParseError::InvalidLine(1));
        assert_eq!(
            parse("cpu_alert=abc").unwrap_err(),
            ParseError::InvalidValue(1, "abc".into())
        );
        assert_eq!(
            parse("cpu_alert=10001").unwrap_err(),
            ParseError::OutOfRange(1)
        );
        assert_eq!(
            parse("frobnicate=1").unwrap_err(),
            ParseError::UnknownKey(1, "frobnicate".into())
        );
        assert_eq!(
            parse("history_enabled=quizas").unwrap_err(),
            ParseError::InvalidValue(1, "quizas".into())
        );
    }

    #[test]
    fn texto_guardado_se_puede_releer() {
        let original = Config {
            cpu_alert: 9500,
            history_path: Some(PathBuf::from("历史.log")),
            watch_names: vec!["init".into(), "sshd".into()],
            ..Config::default()
        };
        assert_eq!(parse(&to_text(&original)).unwrap(), original);
    }

    #[test]
    fn historial_por_defecto_esta_junto_a_la_configuracion() {
        let config = Config::default();
        let path = history_file(&config);
        assert_eq!(path.file_name().unwrap(), "history.log");
        assert_eq!(path.parent(), config_path().parent());

        let explicit = Config {
            history_path: Some(PathBuf::from("otro.log")),
            ..Config::default()
        };
        assert_eq!(history_file(&explicit), PathBuf::from("otro.log"));
    }
}
