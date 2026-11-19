//! Salida JSON construida directamente, sin `serde` ni otros crates.
//!
//! Se escribe en un `String` y se vuelca de una sola vez al `Write` recibido.
//! Los porcentajes se emiten como puntos base enteros (0..10000), igual que en
//! la salida legible, para no arrastrar `f64` ni su coste de formato. Cada
//! función es testeable con un `Vec<u8>` y no lee el sistema.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::Path;

use crate::alerts::{Alert, Metric};
use crate::cli::Command;
use crate::collectors::cpu::CpuReport;
use crate::collectors::disk::DiskInfo;
use crate::collectors::memory::MemoryInfo;
use crate::collectors::network::Interface;
use crate::collectors::processes::Process;
use crate::collectors::temperature::Sensor;
use crate::config::Config;
use crate::core::{NAME, VERSION};
use crate::storage::Record;
use crate::watchdog::{Event, Kind};

/// Escribe `value` como cadena JSON, escapando comillas, contrabarra y
/// caracteres de control. El resto de Unicode se deja tal cual: UTF-8 es
/// válido en JSON.
pub fn string(buf: &mut String, value: &str) {
    buf.push('"');
    for ch in value.chars() {
        match ch {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\u{8}' => buf.push_str("\\b"),
            '\u{c}' => buf.push_str("\\f"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            control if control < '\u{20}' => {
                // `write!` sobre `String` nunca falla.
                let _ = write!(buf, "\\u{:04x}", control as u32);
            }
            normal => buf.push(normal),
        }
    }
    buf.push('"');
}

/// Añade la clave de un campo, con la coma correspondiente.
fn key(buf: &mut String, first: &mut bool, name: &str) {
    if !*first {
        buf.push(',');
    }
    *first = false;
    string(buf, name);
    buf.push(':');
}

/// Añade un número. `write!` sobre `String` es infalible.
fn number(buf: &mut String, value: impl std::fmt::Display) {
    let _ = write!(buf, "{value}");
}

/// Añade un booleano.
fn boolean(buf: &mut String, value: bool) {
    buf.push_str(if value { "true" } else { "false" });
}

/// Vuelca el objeto terminado, con su salto de línea final.
fn emit<W: Write>(out: &mut W, buf: &str) -> io::Result<()> {
    out.write_all(buf.as_bytes())
}

fn name_version_fields(buf: &mut String, first: &mut bool) {
    key(buf, first, "name");
    string(buf, NAME);
    key(buf, first, "version");
    string(buf, VERSION);
}

fn version_object(buf: &mut String) {
    buf.push('{');
    let mut first = true;
    name_version_fields(buf, &mut first);
    buf.push('}');
}

/// `version` como objeto.
pub fn version<W: Write>(out: &mut W) -> io::Result<()> {
    let mut buf = String::new();
    version_object(&mut buf);
    buf.push('\n');
    emit(out, &buf)
}

/// `help` como objeto con la lista canónica de comandos.
pub fn help<W: Write>(out: &mut W) -> io::Result<()> {
    let mut buf = String::from("{");
    let mut first = true;
    name_version_fields(&mut buf, &mut first);
    key(&mut buf, &mut first, "commands");
    buf.push('[');
    for (index, command) in Command::ALL.iter().enumerate() {
        if index > 0 {
            buf.push(',');
        }
        string(&mut buf, command.as_str());
    }
    buf.push(']');
    buf.push('}');
    buf.push('\n');
    emit(out, &buf)
}

fn cpu_object(buf: &mut String, report: &CpuReport) {
    buf.push('{');
    let mut first = true;
    key(buf, &mut first, "usage_basis_points");
    number(buf, report.usage.basis_points);
    key(buf, &mut first, "core_count");
    number(buf, report.core_count);
    key(buf, &mut first, "cores");
    buf.push('[');
    for (index, usage) in report.core_usages.iter().enumerate() {
        if index > 0 {
            buf.push(',');
        }
        number(buf, usage.basis_points);
    }
    buf.push(']');
    buf.push('}');
}

/// `cpu`: uso agregado, número de núcleos y uso de cada uno.
pub fn cpu<W: Write>(out: &mut W, report: &CpuReport) -> io::Result<()> {
    let mut buf = String::new();
    cpu_object(&mut buf, report);
    buf.push('\n');
    emit(out, &buf)
}

fn memory_object(buf: &mut String, info: MemoryInfo) {
    buf.push('{');
    let mut first = true;
    key(buf, &mut first, "total_bytes");
    number(buf, info.total_bytes);
    key(buf, &mut first, "available_bytes");
    number(buf, info.available_bytes);
    key(buf, &mut first, "used_bytes");
    number(buf, info.used_bytes());
    key(buf, &mut first, "used_basis_points");
    number(buf, info.used_basis_points());
    buf.push('}');
}

/// `memory`: bytes totales, disponibles y usados, más el porcentaje.
pub fn memory<W: Write>(out: &mut W, info: MemoryInfo) -> io::Result<()> {
    let mut buf = String::new();
    memory_object(&mut buf, info);
    buf.push('\n');
    emit(out, &buf)
}

fn disk_object(buf: &mut String, info: DiskInfo) {
    buf.push('{');
    let mut first = true;
    key(buf, &mut first, "volume");
    string(buf, info.label());
    key(buf, &mut first, "total_bytes");
    number(buf, info.total_bytes);
    key(buf, &mut first, "available_bytes");
    number(buf, info.available_bytes);
    key(buf, &mut first, "used_bytes");
    number(buf, info.used_bytes());
    key(buf, &mut first, "used_basis_points");
    number(buf, info.used_basis_points());
    buf.push('}');
}

/// `disk`: espacio del volumen raíz.
pub fn disk<W: Write>(out: &mut W, info: DiskInfo) -> io::Result<()> {
    let mut buf = String::new();
    disk_object(&mut buf, info);
    buf.push('\n');
    emit(out, &buf)
}

fn sensors_object(buf: &mut String, sensors: &[Sensor]) {
    buf.push('{');
    let mut first = true;
    key(buf, &mut first, "sensors");
    buf.push('[');
    for (index, sensor) in sensors.iter().enumerate() {
        if index > 0 {
            buf.push(',');
        }
        buf.push('{');
        let mut inner = true;
        key(buf, &mut inner, "name");
        string(buf, &sensor.name);
        key(buf, &mut inner, "millicelsius");
        number(buf, sensor.millicelsius);
        buf.push('}');
    }
    buf.push(']');
    buf.push('}');
}

/// `temperature`: sensores con su lectura en miligrados Celsius.
pub fn temperature<W: Write>(out: &mut W, sensors: &[Sensor]) -> io::Result<()> {
    let mut buf = String::new();
    sensors_object(&mut buf, sensors);
    buf.push('\n');
    emit(out, &buf)
}

fn interfaces_object(buf: &mut String, interfaces: &[Interface]) {
    buf.push('{');
    let mut first = true;
    key(buf, &mut first, "interfaces");
    buf.push('[');
    for (index, interface) in interfaces.iter().enumerate() {
        if index > 0 {
            buf.push(',');
        }
        buf.push('{');
        let mut inner = true;
        key(buf, &mut inner, "name");
        string(buf, &interface.name);
        key(buf, &mut inner, "rx_bytes");
        number(buf, interface.rx_bytes);
        key(buf, &mut inner, "tx_bytes");
        number(buf, interface.tx_bytes);
        buf.push('}');
    }
    buf.push(']');
    buf.push('}');
}

/// `network`: tráfico acumulado por interfaz.
pub fn network<W: Write>(out: &mut W, interfaces: &[Interface]) -> io::Result<()> {
    let mut buf = String::new();
    interfaces_object(&mut buf, interfaces);
    buf.push('\n');
    emit(out, &buf)
}

fn processes_object(buf: &mut String, processes: &[Process]) {
    buf.push('{');
    let mut first = true;
    key(buf, &mut first, "processes");
    buf.push('[');
    for (index, process) in processes.iter().enumerate() {
        if index > 0 {
            buf.push(',');
        }
        buf.push('{');
        let mut inner = true;
        key(buf, &mut inner, "pid");
        number(buf, process.pid);
        key(buf, &mut inner, "name");
        string(buf, &process.name);
        key(buf, &mut inner, "threads");
        number(buf, process.threads);
        key(buf, &mut inner, "rss_bytes");
        number(buf, process.rss_bytes);
        key(buf, &mut inner, "cpu_basis_points");
        number(buf, process.cpu_basis_points);
        buf.push('}');
    }
    buf.push(']');
    buf.push('}');
}

/// `processes`: PID, nombre, hilos, RSS y uso de CPU.
pub fn processes<W: Write>(out: &mut W, processes: &[Process]) -> io::Result<()> {
    let mut buf = String::new();
    processes_object(&mut buf, processes);
    buf.push('\n');
    emit(out, &buf)
}

/// Devuelve `(nombre, valor, umbral, unidad)` de una alerta.
fn alert_fields(alert: &Alert, config: &Config) -> (&'static str, u64, u64, &'static str) {
    match alert.metric {
        Metric::Cpu => (
            "cpu",
            alert.value,
            u64::from(config.cpu_alert),
            "basis_points",
        ),
        Metric::Memory => (
            "memory",
            alert.value,
            u64::from(config.memory_alert),
            "basis_points",
        ),
        Metric::Disk => (
            "disk",
            alert.value,
            u64::from(config.disk_alert),
            "basis_points",
        ),
        Metric::Temperature => (
            "temperature",
            alert.value,
            config.temperature_alert as u64,
            "millicelsius",
        ),
    }
}

fn alerts_array(buf: &mut String, alerts: &[Alert], config: &Config) {
    buf.push('[');
    for (index, alert) in alerts.iter().enumerate() {
        if index > 0 {
            buf.push(',');
        }
        let (metric, value, threshold, unit) = alert_fields(alert, config);
        buf.push('{');
        let mut first = true;
        key(buf, &mut first, "metric");
        string(buf, metric);
        key(buf, &mut first, "value");
        number(buf, value);
        key(buf, &mut first, "threshold");
        number(buf, threshold);
        key(buf, &mut first, "unit");
        string(buf, unit);
        buf.push('}');
    }
    buf.push(']');
}

/// Datos que forman la salida de `status`.
pub struct StatusView<'a> {
    pub report: &'a CpuReport,
    pub info: MemoryInfo,
    pub disk: DiskInfo,
    pub sensors: Option<&'a [Sensor]>,
    pub interfaces: &'a [Interface],
    pub alerts: &'a [Alert],
    pub config: &'a Config,
}

/// `status`: todas las métricas y las alertas disparadas en un solo objeto.
pub fn status<W: Write>(out: &mut W, view: &StatusView<'_>) -> io::Result<()> {
    let StatusView {
        report,
        info,
        disk,
        sensors,
        interfaces,
        alerts,
        config,
    } = *view;
    let mut buf = String::from("{");
    let mut first = true;
    key(&mut buf, &mut first, "cpu");
    cpu_object(&mut buf, report);
    key(&mut buf, &mut first, "memory");
    memory_object(&mut buf, info);
    key(&mut buf, &mut first, "disk");
    disk_object(&mut buf, disk);
    key(&mut buf, &mut first, "network");
    interfaces_object(&mut buf, interfaces);
    key(&mut buf, &mut first, "temperature");
    match sensors {
        Some(list) => sensors_object(&mut buf, list),
        None => buf.push_str("null"),
    }
    key(&mut buf, &mut first, "alerts");
    alerts_array(&mut buf, alerts, config);
    buf.push_str("}\n");
    emit(out, &buf)
}

/// `config`: valores efectivos y ruta del archivo.
pub fn config<W: Write>(out: &mut W, config: &Config, path: &Path) -> io::Result<()> {
    let mut buf = String::from("{");
    let mut first = true;
    key(&mut buf, &mut first, "path");
    string(&mut buf, &path.to_string_lossy());
    key(&mut buf, &mut first, "cpu_alert");
    number(&mut buf, config.cpu_alert);
    key(&mut buf, &mut first, "memory_alert");
    number(&mut buf, config.memory_alert);
    key(&mut buf, &mut first, "disk_alert");
    number(&mut buf, config.disk_alert);
    key(&mut buf, &mut first, "temperature_alert");
    number(&mut buf, config.temperature_alert);
    key(&mut buf, &mut first, "history_enabled");
    boolean(&mut buf, config.history_enabled);
    key(&mut buf, &mut first, "history_max");
    number(&mut buf, config.history_max);
    key(&mut buf, &mut first, "history_path");
    match &config.history_path {
        Some(value) => string(&mut buf, &value.to_string_lossy()),
        None => buf.push_str("null"),
    }
    key(&mut buf, &mut first, "watch_interval_ms");
    number(&mut buf, config.watch_interval_ms);
    key(&mut buf, &mut first, "watch_ticks");
    number(&mut buf, config.watch_ticks);
    key(&mut buf, &mut first, "watch_names");
    buf.push('[');
    for (index, name) in config.watch_names.iter().enumerate() {
        if index > 0 {
            buf.push(',');
        }
        string(&mut buf, name);
    }
    buf.push(']');
    buf.push_str("}\n");
    emit(out, &buf)
}

/// `config path` y `config init`: sólo la ruta relevante.
pub fn config_path<W: Write>(out: &mut W, path: &Path) -> io::Result<()> {
    let mut buf = String::from("{\"path\":");
    string(&mut buf, &path.to_string_lossy());
    buf.push_str("}\n");
    emit(out, &buf)
}

fn record_object(buf: &mut String, record: &Record) {
    buf.push('{');
    let mut first = true;
    key(buf, &mut first, "timestamp");
    number(buf, record.timestamp);
    key(buf, &mut first, "cpu_basis_points");
    number(buf, record.cpu_basis_points);
    key(buf, &mut first, "memory_basis_points");
    number(buf, record.memory_basis_points);
    key(buf, &mut first, "disk_basis_points");
    number(buf, record.disk_basis_points);
    key(buf, &mut first, "temperature_millicelsius");
    match record.temperature_millicelsius {
        Some(value) => number(buf, value),
        None => buf.push_str("null"),
    }
    key(buf, &mut first, "interfaces");
    number(buf, record.interfaces);
    buf.push('}');
}

/// `history`: muestras de antigua a reciente.
pub fn history<W: Write>(out: &mut W, records: &[Record]) -> io::Result<()> {
    let mut buf = String::from("{\"records\":[");
    for (index, record) in records.iter().enumerate() {
        if index > 0 {
            buf.push(',');
        }
        record_object(&mut buf, record);
    }
    buf.push_str("]}\n");
    emit(out, &buf)
}

fn names_array(buf: &mut String, names: &[String]) {
    buf.push('[');
    for (index, name) in names.iter().enumerate() {
        if index > 0 {
            buf.push(',');
        }
        string(buf, name);
    }
    buf.push(']');
}

/// Cabecera de `watch` en JSON Lines.
pub fn watch_start<W: Write>(out: &mut W, names: &[String], interval_ms: u64) -> io::Result<()> {
    let mut buf = String::from("{\"event\":\"start\",\"names\":");
    names_array(&mut buf, names);
    buf.push_str(",\"interval_ms\":");
    number(&mut buf, interval_ms);
    buf.push_str("}\n");
    emit(out, &buf)
}

/// Un evento de `watch`, en su propia línea JSON.
pub fn watch_event<W: Write>(out: &mut W, event: &Event) -> io::Result<()> {
    let kind = match event.kind {
        Kind::Started => "started",
        Kind::Stopped => "stopped",
    };
    let mut buf = String::from("{\"event\":");
    string(&mut buf, kind);
    buf.push_str(",\"name\":");
    string(&mut buf, &event.name);
    buf.push_str(",\"pid\":");
    number(&mut buf, event.pid);
    buf.push_str("}\n");
    emit(out, &buf)
}

#[cfg(test)]
mod tests {
    use super::{
        help, history, memory, processes, status, string, version, watch_event, watch_start,
        StatusView,
    };
    use crate::alerts::{Alert, Metric};
    use crate::cli::Command;
    use crate::collectors::cpu::{CpuReport, CpuUsage};
    use crate::collectors::disk::DiskInfo;
    use crate::collectors::memory::MemoryInfo;
    use crate::collectors::network::Interface;
    use crate::collectors::processes::Process;
    use crate::collectors::temperature::Sensor;
    use crate::config::Config;
    use crate::storage::Record;
    use crate::watchdog::{Event, Kind};

    /// Comprueba que todas las líneas son objetos JSON bien balanceados,
    /// respetando comillas y secuencias de escape. No sustituye a un parser,
    /// pero detecta comas, cierres y escapados mal puestos.
    fn well_formed(text: &str) -> bool {
        for line in text.lines() {
            let mut depth = 0i32;
            let mut quoted = false;
            let mut escaped = false;
            for ch in line.chars() {
                if quoted {
                    if escaped {
                        escaped = false;
                    } else if ch == '\\' {
                        escaped = true;
                    } else if ch == '"' {
                        quoted = false;
                    }
                    continue;
                }
                match ch {
                    '"' => quoted = true,
                    '{' | '[' => depth += 1,
                    '}' | ']' => {
                        depth -= 1;
                        if depth < 0 {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
            if quoted || depth != 0 || !line.starts_with('{') || !line.ends_with('}') {
                return false;
            }
        }
        true
    }

    fn report() -> CpuReport {
        CpuReport {
            usage: CpuUsage { basis_points: 5000 },
            core_usages: vec![CpuUsage { basis_points: 2500 }, CpuUsage::default()],
            core_count: 2,
        }
    }

    #[test]
    fn escapa_caracteres_especiales() {
        let mut buf = String::new();
        string(&mut buf, "a\"b\\c\n\t\u{1}\u{7f}ñ");
        assert_eq!(buf, "\"a\\\"b\\\\c\\n\\t\\u0001\u{7f}ñ\"");
    }

    #[test]
    fn version_y_help_son_objetos_validos() {
        let mut buf = Vec::new();
        version(&mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(well_formed(&text));
        assert_eq!(text, "{\"name\":\"picowatch\",\"version\":\"0.1.0\"}\n");

        let mut buf = Vec::new();
        help(&mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(well_formed(&text));
        assert!(text.contains("\"commands\":[\"status\",\"cpu\""));
        assert!(text.contains("\"help\"]"));
        assert_eq!(text.matches("\"name\"").count(), 1);
        for command in Command::ALL {
            assert!(text.contains(&format!("\"{}\"", command.as_str())));
        }
    }

    #[test]
    fn cpu_usa_enteros_y_no_flotantes() {
        let mut buf = Vec::new();
        super::cpu(&mut buf, &report()).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(well_formed(&text));
        assert!(text.contains("\"usage_basis_points\":5000"));
        assert!(text.contains("\"cores\":[2500,0]"));
        assert!(!text.contains('.'));
    }

    #[test]
    fn memoria_y_disco_incluyen_campos_derivados() {
        let info = MemoryInfo {
            total_bytes: 200,
            available_bytes: 50,
        };
        let mut buf = Vec::new();
        memory(&mut buf, info).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(well_formed(&text));
        assert!(text.contains("\"used_bytes\":150"));
        assert!(text.contains("\"used_basis_points\":7500"));

        let mut buf = Vec::new();
        super::disk(
            &mut buf,
            DiskInfo {
                total_bytes: 200,
                available_bytes: 50,
            },
        )
        .unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(well_formed(&text));
        assert!(text.contains("\"volume\":\"/\""));
    }

    #[test]
    fn sensores_red_y_procesos_son_listas() {
        let mut buf = Vec::new();
        super::temperature(
            &mut buf,
            &[Sensor {
                name: "thermal_zone0".into(),
                millicelsius: 45_200,
            }],
        )
        .unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(well_formed(&text));
        assert!(text.contains("\"millicelsius\":45200"));

        let mut buf = Vec::new();
        super::network(
            &mut buf,
            &[Interface {
                name: "eth0".into(),
                rx_bytes: 1,
                tx_bytes: 2,
            }],
        )
        .unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(well_formed(&text));
        assert!(text.contains("\"rx_bytes\":1,\"tx_bytes\":2"));

        let mut buf = Vec::new();
        processes(
            &mut buf,
            &[Process {
                pid: 42,
                name: "init".into(),
                threads: 3,
                rss_bytes: 4096,
                cpu_basis_points: 1000,
            }],
        )
        .unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(well_formed(&text));
        assert!(text.contains("\"pid\":42,\"name\":\"init\""));
    }

    #[test]
    fn status_reune_todas_las_secciones() {
        let alerts = [
            Alert {
                metric: Metric::Cpu,
                value: 9500,
            },
            Alert {
                metric: Metric::Temperature,
                value: 85_000,
            },
        ];
        let sensors = [Sensor {
            name: "thermal_zone0".into(),
            millicelsius: 85_000,
        }];
        let settings = Config::default();
        let report = report();
        let mut buf = Vec::new();
        status(
            &mut buf,
            &StatusView {
                report: &report,
                info: MemoryInfo {
                    total_bytes: 200,
                    available_bytes: 50,
                },
                disk: DiskInfo {
                    total_bytes: 200,
                    available_bytes: 50,
                },
                sensors: Some(&sensors),
                interfaces: &[],
                alerts: &alerts,
                config: &settings,
            },
        )
        .unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(well_formed(&text));
        for section in [
            "\"cpu\":{",
            "\"memory\":{",
            "\"disk\":{",
            "\"network\":{\"interfaces\":[]}",
            "\"temperature\":{\"sensors\":[",
            "\"alerts\":[",
            "\"unit\":\"basis_points\"",
            "\"unit\":\"millicelsius\"",
        ] {
            assert!(text.contains(section), "falta {section} en {text}");
        }
    }

    #[test]
    fn status_sin_sensores_use_null() {
        let settings = Config::default();
        let report = report();
        let mut buf = Vec::new();
        status(
            &mut buf,
            &StatusView {
                report: &report,
                info: MemoryInfo::default(),
                disk: DiskInfo::default(),
                sensors: None,
                interfaces: &[],
                alerts: &[],
                config: &settings,
            },
        )
        .unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(well_formed(&text));
        assert!(text.contains("\"temperature\":null"));
    }

    #[test]
    fn configuracion_incluye_ruta_y_valores() {
        let config = Config {
            cpu_alert: 8500,
            history_enabled: false,
            watch_names: vec!["init".into(), "a\"b".into()],
            ..Config::default()
        };
        let mut buf = Vec::new();
        super::config(
            &mut buf,
            &config,
            std::path::Path::new("C:\\cfg\\config.txt"),
        )
        .unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(well_formed(&text));
        assert!(text.contains("\"path\":\"C:\\\\cfg\\\\config.txt\""));
        assert!(text.contains("\"cpu_alert\":8500"));
        assert!(text.contains("\"history_enabled\":false"));
        assert!(text.contains("\"history_path\":null"));
        assert!(text.contains("\"a\\\"b\""));

        let mut buf = Vec::new();
        super::config_path(&mut buf, std::path::Path::new("/tmp/config")).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "{\"path\":\"/tmp/config\"}\n"
        );
    }

    #[test]
    fn historial_serializa_temperatura_ausente() {
        let records = [
            Record {
                timestamp: 100,
                cpu_basis_points: 1234,
                memory_basis_points: 5000,
                disk_basis_points: 7500,
                temperature_millicelsius: Some(45_200),
                interfaces: 3,
            },
            Record {
                timestamp: 101,
                cpu_basis_points: 0,
                memory_basis_points: 0,
                disk_basis_points: 0,
                temperature_millicelsius: None,
                interfaces: 0,
            },
        ];
        let mut buf = Vec::new();
        history(&mut buf, &records).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(well_formed(&text));
        assert!(text.contains("\"temperature_millicelsius\":45200"));
        assert!(text.contains("\"temperature_millicelsius\":null"));
        assert_eq!(text.matches("\"timestamp\"").count(), 2);

        let mut buf = Vec::new();
        history(&mut buf, &[]).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "{\"records\":[]}\n");
    }

    #[test]
    fn watch_usa_json_lines() {
        let mut buf = Vec::new();
        watch_start(&mut buf, &["explorer.exe".into()], 50).unwrap();
        watch_event(
            &mut buf,
            &Event {
                kind: Kind::Started,
                name: "explorer.exe".into(),
                pid: 2816,
            },
        )
        .unwrap();
        watch_event(
            &mut buf,
            &Event {
                kind: Kind::Stopped,
                name: "explorer.exe".into(),
                pid: 2816,
            },
        )
        .unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert_eq!(text.lines().count(), 3);
        assert!(well_formed(&text));
        assert_eq!(
            text.lines().next().unwrap(),
            "{\"event\":\"start\",\"names\":[\"explorer.exe\"],\"interval_ms\":50}"
        );
        assert!(text.contains("\"event\":\"started\",\"name\":\"explorer.exe\",\"pid\":2816"));
        assert!(text.contains("\"event\":\"stopped\""));
    }
}
