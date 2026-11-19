//! Espacio de disco de la partición raíz (Windows: `C:\`).
//!
//! `/proc/diskstats` informa de operaciones de E/S, no de capacidad, así que
//! para espacio se usa `statvfs` en Linux y `GetDiskFreeSpaceExW` en Windows.
//! Ambos se declaran directamente: no hay crate `libc` ni llamadas a `df`.

use std::io;

/// Espacio de la partición consultada, en bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

impl DiskInfo {
    pub fn used_bytes(self) -> u64 {
        self.total_bytes.saturating_sub(self.available_bytes)
    }

    pub fn used_basis_points(self) -> u32 {
        if self.total_bytes == 0 {
            return 0;
        }
        let used = self.used_bytes() as u128 * 10_000 / self.total_bytes as u128;
        used.min(10_000) as u32
    }

    /// Etiqueta del volumen en la salida legible.
    pub const fn label(self) -> &'static str {
        "/"
    }
}

#[cfg(target_os = "linux")]
pub fn read() -> io::Result<DiskInfo> {
    let (total, available) = sys::space("/")?;
    Ok(DiskInfo {
        total_bytes: total,
        available_bytes: available,
    })
}

#[cfg(target_os = "windows")]
pub fn read() -> io::Result<DiskInfo> {
    let (total, available) = crate::platform::windows::disk_bytes()?;
    Ok(DiskInfo {
        total_bytes: total,
        available_bytes: available,
    })
}

/// Declaración de `statvfs` tal como la define glibc.
///
/// Los seis enteros finales son el campo de reserva de glibc: hay que
/// incluirlos para que el buffer tenga el tamaño que la función espera.
#[cfg(target_os = "linux")]
mod sys {
    use std::ffi::CString;
    use std::io;
    use std::os::raw::{c_char, c_int, c_uint, c_ulong};

    /// Layout de `struct statvfs` de glibc (bits/statvfs.h).
    #[repr(C)]
    struct Statvfs {
        bsize: c_ulong,
        frsize: c_ulong,
        blocks: c_ulong,
        _bfree: c_ulong,
        bavail: c_ulong,
        _files: c_ulong,
        _ffree: c_ulong,
        _favail: c_ulong,
        _fsid: c_ulong,
        #[cfg(target_pointer_width = "32")]
        _unused: c_int,
        _flag: c_ulong,
        _namemax: c_ulong,
        _file_type: c_uint,
        _spare: [c_int; 5],
    }

    extern "C" {
        fn statvfs(path: *const c_char, buf: *mut Statvfs) -> c_int;
    }

    pub(super) fn space(path: &str) -> io::Result<(u64, u64)> {
        let path = CString::new(path)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        // SAFETY: `info` es una estructura alineada del tamaño exacto de
        // `struct statvfs`; `statvfs` sólo la rellena y no guarda el puntero.
        let mut info: Statvfs = unsafe { std::mem::zeroed() };
        let result = unsafe { statvfs(path.as_ptr(), &mut info) };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        let unit = if info.frsize != 0 {
            info.frsize as u64
        } else {
            info.bsize as u64
        };
        Ok((unit * info.blocks as u64, unit * info.bavail as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::DiskInfo;

    #[test]
    fn calcula_uso_y_porcentaje() {
        let info = DiskInfo {
            total_bytes: 200,
            available_bytes: 50,
        };
        assert_eq!(info.used_bytes(), 150);
        assert_eq!(info.used_basis_points(), 7500);
    }

    #[test]
    fn disco_vacio_no_divide_por_cero() {
        assert_eq!(DiskInfo::default().used_basis_points(), 0);
    }
}
