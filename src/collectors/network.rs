//! Tráfico de red por interfaz.
//!
//! Linux: `/proc/net/dev`. Windows: `GetIfTable` de IP Helper, declarado
//! directamente en `platform::windows`.

use std::io;

/// Contadores acumulados de una interfaz.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// Error de parseo de `/proc/net/dev`.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    InvalidValue,
}

#[cfg(any(target_os = "linux", test))]
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("contador de red no numérico")
    }
}

/// Parsea el contenido de `/proc/net/dev`.
#[cfg(any(target_os = "linux", test))]
pub fn parse_netdev(text: &str) -> Result<Vec<Interface>, ParseError> {
    let mut interfaces = Vec::new();

    for line in text.lines() {
        let (name, data) = match line.split_once(':') {
            Some(parts) => parts,
            None => continue, // cabeceras "Inter-|" y "face |"
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let mut fields = data.split_whitespace();
        let rx = fields
            .next()
            .ok_or(ParseError::InvalidValue)?
            .parse::<u64>()
            .map_err(|_| ParseError::InvalidValue)?;
        // Entre RX bytes y TX bytes hay 7 contadores: packets, errs, drop,
        // fifo, frame, compressed y multicast.
        for _ in 0..7 {
            fields.next();
        }
        let tx = fields
            .next()
            .ok_or(ParseError::InvalidValue)?
            .parse::<u64>()
            .map_err(|_| ParseError::InvalidValue)?;
        interfaces.push(Interface {
            name: name.to_owned(),
            rx_bytes: rx,
            tx_bytes: tx,
        });
    }
    Ok(interfaces)
}

#[cfg(target_os = "linux")]
pub fn read() -> io::Result<Vec<Interface>> {
    let text = std::fs::read_to_string("/proc/net/dev")?;
    parse_netdev(&text)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

#[cfg(target_os = "windows")]
pub fn read() -> io::Result<Vec<Interface>> {
    crate::platform::windows::interfaces()
}

#[cfg(test)]
mod tests {
    use super::{parse_netdev, Interface, ParseError};

    const NETDEV: &str = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 12345 100 0 0 0 0 0 0 67890 100 0 0 0 0 0 0
  eth0: 1000000 500 0 0 0 0 0 0 2000000 400 0 0 0 0 0 0
";

    #[test]
    fn parsea_interfaces() {
        let parsed = parse_netdev(NETDEV).unwrap();
        assert_eq!(
            parsed,
            vec![
                Interface {
                    name: "lo".into(),
                    rx_bytes: 12345,
                    tx_bytes: 67890
                },
                Interface {
                    name: "eth0".into(),
                    rx_bytes: 1_000_000,
                    tx_bytes: 2_000_000
                },
            ]
        );
    }

    #[test]
    fn rechaza_contadores_no_numericos() {
        assert_eq!(
            parse_netdev("eth0: x 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n"),
            Err(ParseError::InvalidValue)
        );
    }
}
