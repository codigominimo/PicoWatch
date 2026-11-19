//! Colectores de métricas del sistema.
//!
//! Cada colector expone una función de parseo pura sobre el contenido de un
//! archivo de `/proc`, más una función `read()` que lo lee. Así las pruebas
//! no dependen de que el equipo de desarrollo sea Linux.

pub mod cpu;
pub mod disk;
pub mod memory;
pub mod network;
pub mod temperature;
