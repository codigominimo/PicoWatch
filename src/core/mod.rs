//! Constantes compartidas por todos los módulos.
//!
//! Se mantiene deliberadamente pequeño: sólo datos compile-time, sin estado.

/// Nombre del ejecutable, usado en la salida y en mensajes de error.
pub const NAME: &str = "picowatch";

/// Versión del paquete, inyectada por Cargo en tiempo de compilación.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
