//! Vigilancia de procesos: eventos de aparición y desaparición.
//!
//! La comparación de muestras es pura (`observed` + `diff`), así que se prueba
//! sin procesos reales ni esperas. `run` sólo inyecta la lectura y el reposo.
//! No hay reinicios automáticos: éstos pertenecen al historial y alertas de la
//! Fase 5 y exigirían crear procesos.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::thread;
use std::time::Duration;

use crate::collectors::processes::Process;

/// Tipo de cambio observado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Started,
    Stopped,
}

/// Cambio de estado de un proceso vigilado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: Kind,
    pub name: String,
    pub pid: u32,
}

/// Configuración de una sesión de vigilancia.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub names: Vec<String>,
    pub interval: Duration,
    /// Número de muestras; `0` significa hasta la interrupción del proceso.
    pub ticks: u32,
}

/// Procesos vigilados presentes en una muestra, indexados por nombre.
pub type Observed = BTreeMap<String, u32>;

/// Filtra una muestra y conserva el último PID conocido por cada nombre.
///
/// Si hay varios procesos con el mismo nombre gana el último de la lista; el
/// evento informa de la existencia del proceso, no de cada instancia.
pub fn observed(processes: &[Process], watched: &[String]) -> Observed {
    let mut result = Observed::new();
    for process in processes {
        if watched.iter().any(|name| name == &process.name) {
            result.insert(process.name.clone(), process.pid);
        }
    }
    result
}

/// Calcula los eventos entre dos muestras, ordenados por nombre.
pub fn diff(previous: &Observed, current: &Observed) -> Vec<Event> {
    let mut events = Vec::new();
    for (name, pid) in current {
        if !previous.contains_key(name) {
            events.push(Event {
                kind: Kind::Started,
                name: name.clone(),
                pid: *pid,
            });
        }
    }
    for (name, pid) in previous {
        if !current.contains_key(name) {
            events.push(Event {
                kind: Kind::Stopped,
                name: name.clone(),
                pid: *pid,
            });
        }
    }
    events
}

/// Ejecuta la vigilancia `config.ticks` muestras (o indefinidamente si es 0).
///
/// La lectura, el reposo y el formato de cada evento se inyectan: las pruebas
/// no duermen y `main` elige entre salida legible y JSON sin duplicar el bucle.
pub fn run<W, C, S, E>(
    config: &Config,
    mut collect: C,
    mut sleep: S,
    mut emit: E,
    out: &mut W,
) -> io::Result<()>
where
    W: Write,
    C: FnMut() -> io::Result<Vec<Process>>,
    S: FnMut(Duration),
    E: FnMut(&mut W, &Event) -> io::Result<()>,
{
    let mut previous = Observed::new();
    let mut ticks = 0u32;
    loop {
        let current = observed(&collect()?, &config.names);
        for event in diff(&previous, &current) {
            emit(out, &event)?;
        }
        previous = current;

        ticks += 1;
        if config.ticks > 0 && ticks >= config.ticks {
            return Ok(());
        }
        sleep(config.interval);
    }
}

/// Configuración lista para `run`, derivada de la CLI.
pub fn from_options(options: crate::cli::WatchOptions) -> Config {
    Config {
        names: options.names,
        interval: Duration::from_millis(options.interval_ms),
        ticks: options.ticks,
    }
}

/// Adaptador para `main`: usa el colector real y `thread::sleep`.
pub fn run_system_with<W, E>(config: &Config, emit: E, out: &mut W) -> io::Result<()>
where
    W: Write,
    E: FnMut(&mut W, &Event) -> io::Result<()>,
{
    run(
        config,
        crate::collectors::processes::list,
        thread::sleep,
        emit,
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::{diff, from_options, observed, run, Config, Event, Kind, Observed};
    use crate::cli::WatchOptions;
    use crate::collectors::processes::Process;
    use crate::output::terminal;
    use std::time::Duration;

    fn process(pid: u32, name: &str) -> Process {
        Process {
            pid,
            name: name.into(),
            threads: 1,
            rss_bytes: 0,
            cpu_basis_points: 0,
        }
    }

    #[test]
    fn filtra_solo_los_nombres_vigilados() {
        let processes = [process(1, "init"), process(2, "sshd"), process(3, "otro")];
        let watched = vec!["init".to_string(), "sshd".to_string()];
        let result = observed(&processes, &watched);
        assert_eq!(result.get("init"), Some(&1));
        assert_eq!(result.get("sshd"), Some(&2));
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn detecta_inicios_y_paradas() {
        let mut before = Observed::new();
        before.insert("sshd".to_string(), 10);
        let mut after = Observed::new();
        after.insert("init".to_string(), 20);

        assert_eq!(
            diff(&before, &after),
            vec![
                Event {
                    kind: Kind::Started,
                    name: "init".into(),
                    pid: 20,
                },
                Event {
                    kind: Kind::Stopped,
                    name: "sshd".into(),
                    pid: 10,
                },
            ]
        );
    }

    #[test]
    fn sin_cambios_no_hay_eventos() {
        let mut sample = Observed::new();
        sample.insert("init".to_string(), 1);
        assert!(diff(&sample, &sample).is_empty());
    }

    #[test]
    fn run_emite_eventos_y_respeta_ticks() {
        let config = Config {
            names: vec!["init".into()],
            interval: Duration::from_millis(5),
            ticks: 3,
        };
        let samples = [vec![process(1, "init")], vec![process(1, "init")], vec![]];
        let mut index = 0;
        let mut slept = Vec::new();
        let mut out = Vec::new();

        run(
            &config,
            || {
                let sample = samples[index.min(samples.len() - 1)].clone();
                index += 1;
                Ok(sample)
            },
            |duration| slept.push(duration),
            terminal::watch_event,
            &mut out,
        )
        .unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("+ iniciado: init"), "salida: {text}");
        assert!(text.contains("- detenido: init"), "salida: {text}");
        assert_eq!(slept.len(), 2);
        assert_eq!(index, 3);
    }

    #[test]
    fn convierte_opciones_de_la_cli() {
        let config = from_options(WatchOptions {
            names: vec!["init".into()],
            interval_ms: 250,
            ticks: 4,
        });
        assert_eq!(config.interval, Duration::from_millis(250));
        assert_eq!(config.ticks, 4);
    }
}
