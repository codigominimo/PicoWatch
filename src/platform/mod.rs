//! Código específico del sistema operativo.
//!
//! En Linux los colectores leen `/proc` y `/sys` directamente con `std::fs`.
//! En Windows se usa un enlazado mínimo a la API del sistema: `std` no expone
//! contadores de CPU, memoria, disco ni interfaces, y la alternativa de
//! lanzar `wmic`/PowerShell sería lenta y frágil.

#[cfg(target_os = "windows")]
pub mod windows;
