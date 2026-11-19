//! Analizador de línea de comandos sin dependencias externas.
//!
//! `parse` es una función pura sobre un iterador, por lo que se puede probar
//! sin leer `std::env`. En la ruta correcta no realiza ninguna asignación:
//! compara `&OsStr` directamente y sólo copia datos en la ruta de error.

use std::ffi::{OsStr, OsString};
use std::fmt;

/// Comandos reconocidos por la CLI.
///
/// `processes`, `watch` y `config` se aceptan para fijar la interfaz pública,
/// pero se implementan en fases posteriores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    Cpu,
    Memory,
    Disk,
    Temperature,
    Network,
    Processes,
    Watch,
    Config,
    Version,
    Help,
}

impl Command {
    /// Representación canónica del comando (también usada por `Display`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Disk => "disk",
            Self::Temperature => "temperature",
            Self::Network => "network",
            Self::Processes => "processes",
            Self::Watch => "watch",
            Self::Config => "config",
            Self::Version => "version",
            Self::Help => "help",
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Errores de sintaxis de la línea de comandos.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    UnknownCommand(OsString),
    UnexpectedArgument(OsString),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCommand(cmd) => {
                write!(f, "comando desconocido: {}", cmd.to_string_lossy())
            }
            Self::UnexpectedArgument(arg) => {
                write!(f, "argumento inesperado: {}", arg.to_string_lossy())
            }
        }
    }
}

/// Parsea los argumentos **sin** incluir el nombre del programa.
///
/// Sin argumentos se muestra la ayuda, comportamiento habitual en CLI.
pub fn parse<I, S>(args: I) -> Result<Command, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut iter = args.into_iter();
    let first = match iter.next() {
        Some(arg) => arg,
        None => return Ok(Command::Help),
    };

    let command = match first.as_ref() {
        arg if arg == "status" => Command::Status,
        arg if arg == "cpu" => Command::Cpu,
        arg if arg == "memory" => Command::Memory,
        arg if arg == "disk" => Command::Disk,
        arg if arg == "temperature" => Command::Temperature,
        arg if arg == "network" => Command::Network,
        arg if arg == "processes" => Command::Processes,
        arg if arg == "watch" => Command::Watch,
        arg if arg == "config" => Command::Config,
        arg if arg == "version" || arg == "--version" || arg == "-V" => Command::Version,
        arg if arg == "help" || arg == "--help" || arg == "-h" => Command::Help,
        other => return Err(ParseError::UnknownCommand(other.to_os_string())),
    };

    if let Some(extra) = iter.next() {
        return Err(ParseError::UnexpectedArgument(
            extra.as_ref().to_os_string(),
        ));
    }

    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::{parse, Command, ParseError};

    #[test]
    fn sin_argumentos_muestra_ayuda() {
        assert_eq!(parse(std::iter::empty::<&str>()), Ok(Command::Help));
    }

    #[test]
    fn reconoce_todos_los_comandos() {
        let casos = [
            ("status", Command::Status),
            ("cpu", Command::Cpu),
            ("memory", Command::Memory),
            ("disk", Command::Disk),
            ("temperature", Command::Temperature),
            ("network", Command::Network),
            ("processes", Command::Processes),
            ("watch", Command::Watch),
            ("config", Command::Config),
            ("version", Command::Version),
            ("help", Command::Help),
        ];
        for (entrada, esperado) in casos {
            assert_eq!(parse([entrada]), Ok(esperado), "comando {entrada}");
        }
    }

    #[test]
    fn alias_de_ayuda_y_version() {
        for alias in ["--help", "-h"] {
            assert_eq!(parse([alias]), Ok(Command::Help));
        }
        for alias in ["--version", "-V"] {
            assert_eq!(parse([alias]), Ok(Command::Version));
        }
    }

    #[test]
    fn rechaza_comando_desconocido() {
        let error = parse(["frobnicate"]).unwrap_err();
        assert_eq!(
            error,
            ParseError::UnknownCommand("frobnicate".into()),
            "error: {error}"
        );
    }

    #[test]
    fn rechaza_argumentos_extra() {
        let error = parse(["cpu", "--json"]).unwrap_err();
        assert_eq!(
            error,
            ParseError::UnexpectedArgument("--json".into()),
            "error: {error}"
        );
    }

    #[test]
    fn display_usa_la_forma_canonica() {
        assert_eq!(Command::Temperature.to_string(), "temperature");
    }

    #[test]
    fn mensajes_de_error_son_legibles() {
        assert_eq!(
            ParseError::UnknownCommand("x".into()).to_string(),
            "comando desconocido: x"
        );
        assert_eq!(
            ParseError::UnexpectedArgument("y".into()).to_string(),
            "argumento inesperado: y"
        );
    }
}
