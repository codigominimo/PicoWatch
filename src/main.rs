//! Punto de entrada de PicoWatch.
//!
//! Sin argumentos abre el panel interactivo (ratatui); los comandos siguen
//! disponibles para guiones, con salida legible o JSON (`--json`).

mod alerts;
mod cli;
mod collectors;
mod config;
mod core;
mod output;
mod platform;
mod storage;
mod tui;
mod watchdog;

use std::fmt;
use std::io::{self, Write};
use std::process::ExitCode;

use cli::{Command, ParseError};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::Usage(error)) => {
            let stderr = io::stderr();
            let mut err = stderr.lock();
            let _ = writeln!(err, "{}: {error}", core::NAME);
            let _ = writeln!(err, "Use '{} help' para ver los comandos.", core::NAME);
            ExitCode::from(2)
        }
        Err(Error::Io(error)) => {
            let stderr = io::stderr();
            let mut err = stderr.lock();
            let _ = writeln!(err, "{}: error de E/S: {error}", core::NAME);
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), Error> {
    let invocation = cli::parse(std::env::args_os().skip(1))?;
    let json = invocation.json;

    // El TUI toma posesión de la terminal: se abre antes de bloquear `stdout`.
    if invocation.command == Command::Tui {
        return tui::run().map_err(Error::Io);
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();
    match invocation.command {
        // Ya se ha devuelto antes de bloquear `stdout`; se repite para que el
        // match sea exhaustivo sin recurrir a `unreachable!`.
        Command::Tui => tui::run()?,
        Command::Help => {
            if json {
                output::json::help(&mut out)?;
            } else {
                output::terminal::help(&mut out)?;
            }
        }
        Command::Version => {
            if json {
                output::json::version(&mut out)?;
            } else {
                output::terminal::version(&mut out)?;
            }
        }
        Command::Cpu => {
            let report = collectors::cpu::sample(collectors::cpu::DEFAULT_INTERVAL)?;
            if json {
                output::json::cpu(&mut out, &report)?;
            } else {
                output::terminal::cpu(&mut out, &report)?;
            }
        }
        Command::Memory => {
            let info = collectors::memory::read()?;
            if json {
                output::json::memory(&mut out, info)?;
            } else {
                output::terminal::memory(&mut out, info)?;
            }
        }
        Command::Disk => {
            let info = collectors::disk::read()?;
            if json {
                output::json::disk(&mut out, info)?;
            } else {
                output::terminal::disk(&mut out, info)?;
            }
        }
        Command::Temperature => {
            let sensors = collectors::temperature::read()?;
            if json {
                output::json::temperature(&mut out, &sensors)?;
            } else {
                output::terminal::temperature(&mut out, &sensors)?;
            }
        }
        Command::Network => {
            let interfaces = collectors::network::read()?;
            if json {
                output::json::network(&mut out, &interfaces)?;
            } else {
                output::terminal::network(&mut out, &interfaces)?;
            }
        }
        Command::Status => {
            let report = collectors::cpu::sample(collectors::cpu::DEFAULT_INTERVAL)?;
            let info = collectors::memory::read()?;
            let disk_info = collectors::disk::read()?;
            // Windows no ofrece sensores y en Linux/sysfs pueden no existir:
            // el resumen sigue siendo útil sin temperatura.
            let sensors = collectors::temperature::read().ok();
            let interfaces = collectors::network::read()?;

            let settings = config::load()?;
            let max_temperature = sensors
                .as_ref()
                .and_then(|list| list.iter().map(|sensor| sensor.millicelsius).max());
            let detected = alerts::evaluate(
                &settings,
                report.usage.basis_points,
                info.used_basis_points(),
                disk_info.used_basis_points(),
                max_temperature,
            );

            if settings.history_enabled {
                let record = storage::Record {
                    timestamp: storage::now(),
                    cpu_basis_points: report.usage.basis_points,
                    memory_basis_points: info.used_basis_points(),
                    disk_basis_points: disk_info.used_basis_points(),
                    temperature_millicelsius: max_temperature,
                    interfaces: interfaces.len() as u32,
                };
                storage::append(
                    &config::history_file(&settings),
                    record,
                    settings.history_max,
                )?;
            }

            if json {
                output::json::status(
                    &mut out,
                    &output::json::StatusView {
                        report: &report,
                        info,
                        disk: disk_info,
                        sensors: sensors.as_deref(),
                        interfaces: &interfaces,
                        alerts: &detected,
                        config: &settings,
                    },
                )?;
            } else {
                output::terminal::status(
                    &mut out,
                    &report,
                    info,
                    disk_info,
                    sensors.as_deref(),
                    &interfaces,
                )?;
                output::terminal::alerts(&mut out, &detected, &settings)?;
            }
        }
        Command::Processes => {
            let processes = collectors::processes::sample(collectors::processes::DEFAULT_INTERVAL)?;
            if json {
                output::json::processes(&mut out, &processes)?;
            } else {
                output::terminal::processes(&mut out, &processes)?;
            }
        }
        Command::Watch => {
            let options = cli::parse_watch(invocation.arguments)?;
            let interval_ms = options.interval_ms;
            let settings = watchdog::from_options(options);
            if json {
                output::json::watch_start(&mut out, &settings.names, interval_ms)?;
                watchdog::run_system_with(&settings, output::json::watch_event, &mut out)?;
            } else {
                output::terminal::watch_start(&mut out, &settings.names, interval_ms)?;
                watchdog::run_system_with(&settings, output::terminal::watch_event, &mut out)?;
            }
        }
        Command::Config => match cli::parse_config(invocation.arguments)? {
            cli::ConfigAction::Show => {
                let settings = config::load()?;
                let path = config::config_path();
                if json {
                    output::json::config(&mut out, &settings, &path)?;
                } else {
                    output::terminal::config(&mut out, &settings, &path)?;
                }
            }
            cli::ConfigAction::Path => {
                let path = config::config_path();
                if json {
                    output::json::config_path(&mut out, &path)?;
                } else {
                    writeln!(out, "{}", path.display())?;
                }
            }
            cli::ConfigAction::Init => {
                let path = config::init()?;
                if json {
                    output::json::config_path(&mut out, &path)?;
                } else {
                    writeln!(out, "Configuración creada: {}", path.display())?;
                }
            }
        },
        Command::History => {
            let count = cli::parse_history(invocation.arguments)?;
            let settings = config::load()?;
            let records = storage::read_last(&config::history_file(&settings), count)?;
            if json {
                output::json::history(&mut out, &records)?;
            } else {
                output::terminal::history(&mut out, &records)?;
            }
        }
    }
    Ok(())
}

/// Errores de ejecución. Se distinguen los de uso (exit 2) de los de E/S (exit 1).
enum Error {
    Usage(ParseError),
    Io(io::Error),
}

impl From<ParseError> for Error {
    fn from(error: ParseError) -> Self {
        Self::Usage(error)
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
        }
    }
}
