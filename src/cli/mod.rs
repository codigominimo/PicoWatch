//! Analizador de línea de comandos sin dependencias externas.
//!
//! `parse` y `parse_watch` son funciones puras sobre iteradores, por lo que se
//! pueden probar sin leer `std::env`. En la ruta correcta comparan `&OsStr`
//! directamente y sólo asignan cuando hay argumentos que conservar.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::str::FromStr;

/// Comandos reconocidos por la CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Panel interactivo con gráficas: es la acción por defecto.
    Tui,
    Status,
    Cpu,
    Memory,
    Disk,
    Temperature,
    Network,
    Processes,
    Watch,
    Config,
    History,
    Version,
    Help,
}

impl Command {
    /// Todos los comandos, en el orden en que se muestran en la ayuda.
    pub const ALL: [Self; 13] = [
        Self::Tui,
        Self::Status,
        Self::Cpu,
        Self::Memory,
        Self::Disk,
        Self::Temperature,
        Self::Network,
        Self::Processes,
        Self::Watch,
        Self::Config,
        Self::History,
        Self::Version,
        Self::Help,
    ];

    /// Representación canónica del comando (también usada por `Display`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tui => "tui",
            Self::Status => "status",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Disk => "disk",
            Self::Temperature => "temperature",
            Self::Network => "network",
            Self::Processes => "processes",
            Self::Watch => "watch",
            Self::Config => "config",
            Self::History => "history",
            Self::Version => "version",
            Self::Help => "help",
        }
    }

    /// Comandos que admiten argumentos propios.
    pub const fn takes_arguments(self) -> bool {
        matches!(self, Self::Watch | Self::Config | Self::History)
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Comando, argumentos sin analizar y formato de salida elegido.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub command: Command,
    pub arguments: Vec<OsString>,
    /// `--json` puede ir antes o después del comando.
    pub json: bool,
}

impl From<Command> for Invocation {
    fn from(command: Command) -> Self {
        Self {
            command,
            arguments: Vec::new(),
            json: false,
        }
    }
}

/// Opciones ya validadas del comando `watch`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchOptions {
    /// Nombres de proceso a vigilar (coincidencia exacta).
    pub names: Vec<String>,
    /// Periodo entre muestras, en milisegundos.
    pub interval_ms: u64,
    /// Número de muestras; `0` significa hasta que se interrumpa el proceso.
    pub ticks: u32,
}

/// Acción solicitada por `config`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigAction {
    Show,
    Path,
    Init,
}

/// Errores de sintaxis de la línea de comandos.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    UnknownCommand(OsString),
    UnexpectedArgument(OsString),
    MissingValue(&'static str),
    InvalidValue(OsString, &'static str),
    MissingProcessName,
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
            Self::MissingValue(option) => write!(f, "falta el valor para {option}"),
            Self::InvalidValue(value, option) => {
                write!(
                    f,
                    "valor no válido para {option}: {}",
                    value.to_string_lossy()
                )
            }
            Self::MissingProcessName => f.write_str("watch necesita al menos un nombre de proceso"),
        }
    }
}

/// Parsea los argumentos **sin** incluir el nombre del programa.
///
/// Sin argumentos se abre el panel interactivo (`tui`). La CLI sigue disponible
/// para guiones y entornos sin terminal.
/// `--json` es una opción global: se acepta antes o después del comando y se
/// elimina de los argumentos que recibe cada subcomando. Sólo `watch`,
/// `config` e `history` conservan argumentos propios.
pub fn parse<I, S>(args: I) -> Result<Invocation, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut iter = args.into_iter();
    let mut json = false;
    let first = loop {
        match iter.next() {
            Some(arg) if arg.as_ref() == "--json" => json = true,
            Some(arg) => break arg,
            None => {
                // Sin argumentos se abre directamente el panel interactivo:
                // es la forma habitual de usar un monitor en pantalla completa.
                return Ok(Invocation {
                    command: Command::Tui,
                    arguments: Vec::new(),
                    json,
                });
            }
        }
    };

    let command = match first.as_ref() {
        arg if arg == "tui" || arg == "ui" => Command::Tui,
        arg if arg == "status" => Command::Status,
        arg if arg == "cpu" => Command::Cpu,
        arg if arg == "memory" => Command::Memory,
        arg if arg == "disk" => Command::Disk,
        arg if arg == "temperature" => Command::Temperature,
        arg if arg == "network" => Command::Network,
        arg if arg == "processes" => Command::Processes,
        arg if arg == "watch" => Command::Watch,
        arg if arg == "config" => Command::Config,
        arg if arg == "history" => Command::History,
        arg if arg == "version" || arg == "--version" || arg == "-V" => Command::Version,
        arg if arg == "help" || arg == "--help" || arg == "-h" => Command::Help,
        other => return Err(ParseError::UnknownCommand(other.to_os_string())),
    };

    let mut arguments: Vec<OsString> = Vec::new();
    for arg in iter {
        let raw = arg.as_ref();
        if raw == "--json" {
            json = true;
        } else {
            arguments.push(raw.to_os_string());
        }
    }
    if !command.takes_arguments() {
        if let Some(extra) = arguments.first() {
            return Err(ParseError::UnexpectedArgument(extra.clone()));
        }
    }

    Ok(Invocation {
        command,
        arguments,
        json,
    })
}

fn parse_number<T: FromStr>(arg: &OsStr, option: &'static str) -> Result<T, ParseError> {
    arg.to_str()
        .and_then(|text| text.parse::<T>().ok())
        .ok_or_else(|| ParseError::InvalidValue(arg.to_os_string(), option))
}

/// Interpreta los argumentos de `watch`.
///
/// Sintaxis: `watch [--interval MS] [--ticks N] nombre [nombre...]`.
/// El orden de las opciones y los nombres es libre.
pub fn parse_watch<I, S>(args: I) -> Result<WatchOptions, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut names = Vec::new();
    let mut interval_ms = 1000u64;
    let mut ticks = 0u32;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        let raw = arg.as_ref();
        let text = raw.to_string_lossy();
        match text.as_ref() {
            "--interval" => {
                let value = iter.next().ok_or(ParseError::MissingValue("--interval"))?;
                interval_ms = parse_number(value.as_ref(), "--interval")?;
                if interval_ms == 0 {
                    return Err(ParseError::InvalidValue(
                        value.as_ref().to_os_string(),
                        "--interval",
                    ));
                }
            }
            "--ticks" => {
                let value = iter.next().ok_or(ParseError::MissingValue("--ticks"))?;
                ticks = parse_number(value.as_ref(), "--ticks")?;
            }
            _ => names.push(text.into_owned()),
        }
    }

    if names.is_empty() {
        return Err(ParseError::MissingProcessName);
    }
    Ok(WatchOptions {
        names,
        interval_ms,
        ticks,
    })
}

/// Interpreta los argumentos de `config`.
pub fn parse_config<I, S>(args: I) -> Result<ConfigAction, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut iter = args.into_iter();
    let action = match iter.next() {
        None => ConfigAction::Show,
        Some(arg) => match arg.as_ref() {
            value if value == "show" => ConfigAction::Show,
            value if value == "path" => ConfigAction::Path,
            value if value == "init" => ConfigAction::Init,
            other => return Err(ParseError::UnexpectedArgument(other.to_os_string())),
        },
    };
    if let Some(extra) = iter.next() {
        return Err(ParseError::UnexpectedArgument(
            extra.as_ref().to_os_string(),
        ));
    }
    Ok(action)
}

/// Interpreta los argumentos de `history`: `--last N`.
pub fn parse_history<I, S>(args: I) -> Result<usize, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut iter = args.into_iter();
    let first = match iter.next() {
        None => return Ok(10),
        Some(arg) => arg,
    };
    if first.as_ref() != "--last" {
        return Err(ParseError::UnexpectedArgument(
            first.as_ref().to_os_string(),
        ));
    }
    let value = iter.next().ok_or(ParseError::MissingValue("--last"))?;
    let count: usize = parse_number(value.as_ref(), "--last")?;
    if let Some(extra) = iter.next() {
        return Err(ParseError::UnexpectedArgument(
            extra.as_ref().to_os_string(),
        ));
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::{
        parse, parse_config, parse_history, parse_watch, Command, ConfigAction, Invocation,
        ParseError, WatchOptions,
    };

    #[test]
    fn sin_argumentos_abre_el_panel() {
        assert_eq!(
            parse(std::iter::empty::<&str>()),
            Ok(Invocation::from(Command::Tui))
        );
    }

    #[test]
    fn reconoce_todos_los_comandos() {
        let casos = [
            ("tui", Command::Tui),
            ("ui", Command::Tui),
            ("status", Command::Status),
            ("cpu", Command::Cpu),
            ("memory", Command::Memory),
            ("disk", Command::Disk),
            ("temperature", Command::Temperature),
            ("network", Command::Network),
            ("processes", Command::Processes),
            ("watch", Command::Watch),
            ("config", Command::Config),
            ("history", Command::History),
            ("version", Command::Version),
            ("help", Command::Help),
        ];
        for (entrada, esperado) in casos {
            assert_eq!(
                parse([entrada]),
                Ok(Invocation::from(esperado)),
                "comando {entrada}"
            );
        }
    }

    #[test]
    fn alias_de_ayuda_y_version() {
        for alias in ["--help", "-h"] {
            assert_eq!(parse([alias]), Ok(Invocation::from(Command::Help)));
        }
        for alias in ["--version", "-V"] {
            assert_eq!(parse([alias]), Ok(Invocation::from(Command::Version)));
        }
    }

    #[test]
    fn watch_conserva_sus_argumentos() {
        assert_eq!(
            parse(["watch", "init", "--ticks", "2"]),
            Ok(Invocation {
                command: Command::Watch,
                arguments: vec!["init".into(), "--ticks".into(), "2".into()],
                json: false,
            })
        );
    }

    #[test]
    fn parsea_acciones_de_configuracion() {
        assert_eq!(
            parse_config(std::iter::empty::<&str>()),
            Ok(ConfigAction::Show)
        );
        assert_eq!(parse_config(["show"]), Ok(ConfigAction::Show));
        assert_eq!(parse_config(["path"]), Ok(ConfigAction::Path));
        assert_eq!(parse_config(["init"]), Ok(ConfigAction::Init));
        assert_eq!(
            parse_config(["init", "extra"]),
            Err(ParseError::UnexpectedArgument("extra".into()))
        );
    }

    #[test]
    fn parsea_opciones_de_historial() {
        assert_eq!(parse_history(std::iter::empty::<&str>()), Ok(10));
        assert_eq!(parse_history(["--last", "25"]), Ok(25));
        assert_eq!(
            parse_history(["--last"]),
            Err(ParseError::MissingValue("--last"))
        );
        assert_eq!(
            parse_history(["--last", "x"]),
            Err(ParseError::InvalidValue("x".into(), "--last"))
        );
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
        let error = parse(["cpu", "--foo"]).unwrap_err();
        assert_eq!(
            error,
            ParseError::UnexpectedArgument("--foo".into()),
            "error: {error}"
        );
    }

    #[test]
    fn json_acepta_antes_y_despues_del_comando() {
        for args in [
            vec!["--json", "status"],
            vec!["status", "--json"],
            vec!["--json", "watch", "--json", "--ticks", "1", "init"],
        ] {
            let invocation = parse(args.clone()).unwrap();
            assert!(invocation.json, "argumentos: {args:?}");
            assert!(
                !invocation.arguments.iter().any(|arg| arg == "--json"),
                "--json no debe filtrarse al subcomando"
            );
        }
        assert_eq!(
            parse(["--json"]),
            Ok(Invocation {
                command: Command::Tui,
                arguments: Vec::new(),
                json: true,
            })
        );
        assert!(!parse(["status"]).unwrap().json);
    }

    #[test]
    fn todos_los_comandos_estan_en_la_lista() {
        assert_eq!(Command::ALL.len(), 13);
        for command in Command::ALL {
            assert_eq!(parse([command.as_str()]).unwrap().command, command);
        }
    }

    #[test]
    fn parsea_opciones_de_watch() {
        assert_eq!(
            parse_watch(["--interval", "250", "--ticks", "3", "init", "sshd"]),
            Ok(WatchOptions {
                names: vec!["init".into(), "sshd".into()],
                interval_ms: 250,
                ticks: 3,
            })
        );
    }

    #[test]
    fn watch_usa_valores_por_defecto() {
        assert_eq!(
            parse_watch(["init"]),
            Ok(WatchOptions {
                names: vec!["init".into()],
                interval_ms: 1000,
                ticks: 0,
            })
        );
    }

    #[test]
    fn rechaza_opciones_de_watch_invalidas() {
        assert_eq!(
            parse_watch(["--interval"]),
            Err(ParseError::MissingValue("--interval"))
        );
        assert_eq!(
            parse_watch(["--ticks"]),
            Err(ParseError::MissingValue("--ticks"))
        );
        assert_eq!(
            parse_watch(["--interval", "x", "init"]),
            Err(ParseError::InvalidValue("x".into(), "--interval"))
        );
        assert_eq!(
            parse_watch(["--interval", "0", "init"]),
            Err(ParseError::InvalidValue("0".into(), "--interval"))
        );
        assert_eq!(
            parse_watch(std::iter::empty::<&str>()),
            Err(ParseError::MissingProcessName)
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
        assert_eq!(
            ParseError::MissingValue("--ticks").to_string(),
            "falta el valor para --ticks"
        );
        assert_eq!(
            ParseError::InvalidValue("z".into(), "--ticks").to_string(),
            "valor no válido para --ticks: z"
        );
        assert_eq!(
            ParseError::MissingProcessName.to_string(),
            "watch necesita al menos un nombre de proceso"
        );
    }
}
