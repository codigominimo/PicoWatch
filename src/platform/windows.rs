//! Enlazado mínimo a Win32 (`kernel32`, `iphlpapi`, `ntdll`), sin crates externos.
//!
//! Justificación del `unsafe`: no existe API en `std` para obtener los
//! contadores del sistema. Se usan funciones documentadas por Microsoft, con
//! estructuras `#[repr(C)]` y sin punteros devueltos por la API, de modo que
//! el riesgo se limita a las llamadas FFI siguientes. La única excepción es
//! `NtQuerySystemInformation`: no hay equivalente documentado para los
//! contadores por núcleo, y su clase 8 y su estructura llevan décadas
//! estables (están declaradas en `winternl.h` del SDK).

use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::ptr;

/// `FILETIME`: 100 ns desde 1601-01-01, dividido en dos palabras de 32 bits.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FileTime {
    pub low: u32,
    pub high: u32,
}

impl FileTime {
    pub fn as_u64(self) -> u64 {
        (u64::from(self.high) << 32) | u64::from(self.low)
    }
}

/// `MEMORYSTATUSEX` tal como la espera `GlobalMemoryStatusEx`.
#[repr(C)]
pub struct MemoryStatusEx {
    pub length: u32,
    pub memory_load: u32,
    pub total_phys: u64,
    pub avail_phys: u64,
    pub total_page_file: u64,
    pub avail_page_file: u64,
    pub total_virtual: u64,
    pub avail_virtual: u64,
    pub avail_extended_virtual: u64,
}

#[link(name = "kernel32")]
extern "system" {
    #[link_name = "GetSystemTimes"]
    fn get_system_times(idle: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;

    #[link_name = "GetProcessTimes"]
    fn get_process_times(
        process: *mut c_void,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;

    #[link_name = "GlobalMemoryStatusEx"]
    fn global_memory_status_ex(status: *mut MemoryStatusEx) -> i32;

    #[link_name = "GetDiskFreeSpaceExW"]
    fn get_disk_free_space_ex_w(
        directory: *const u16,
        free_available: *mut u64,
        total: *mut u64,
        total_free: *mut u64,
    ) -> i32;

    #[link_name = "CreateToolhelp32Snapshot"]
    fn create_toolhelp32_snapshot(flags: u32, process_id: u32) -> *mut c_void;

    #[link_name = "Process32FirstW"]
    fn process32_first_w(snapshot: *mut c_void, entry: *mut ProcessEntry32W) -> i32;

    #[link_name = "Process32NextW"]
    fn process32_next_w(snapshot: *mut c_void, entry: *mut ProcessEntry32W) -> i32;

    #[link_name = "OpenProcess"]
    fn open_process(access: u32, inherit_handle: i32, process_id: u32) -> *mut c_void;

    #[link_name = "K32GetProcessMemoryInfo"]
    fn get_process_memory_info(
        process: *mut c_void,
        counters: *mut ProcessMemoryCounters,
        size: u32,
    ) -> i32;

    #[link_name = "CloseHandle"]
    fn close_handle(handle: *mut c_void) -> i32;
}

/// Devuelve `(usuario, kernel, idle)` en ticks de 100 ns.
///
/// `GetSystemTimes` incluye el tiempo `idle` dentro de `kernel`; el colector
/// lo resta para encajarlo en el modelo de `/proc/stat`.
pub fn cpu_times() -> io::Result<(u64, u64, u64)> {
    let mut idle = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();

    // SAFETY: los tres punteros apilan variables inicializadas y válidas
    // durante la llamada; la función sólo las escribe.
    let ok = unsafe { get_system_times(&mut idle, &mut kernel, &mut user) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((user.as_u64(), kernel.as_u64(), idle.as_u64()))
}

/// `SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION` (`winternl.h`): 0x30 bytes en
/// x86, x64 y ARM64. Sólo se consultan los tres primeros contadores; el resto
/// del registro se conserva para mantener el paso de 48 bytes entre entradas.
#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessorPerformance {
    idle: i64,
    kernel: i64,
    user: i64,
    _dpc: i64,
    _interrupt: i64,
    _count: u32,
    _padding: u32,
}

#[link(name = "ntdll")]
extern "system" {
    #[link_name = "NtQuerySystemInformation"]
    fn nt_query_system_information(
        class: u32,
        information: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
}

/// Convierte las entradas devueltas por el kernel a `(usuario, kernel, idle)`.
///
/// Al igual que `GetSystemTimes`, el `KernelTime` de esta estructura incluye
/// el `IdleTime`: se resta para no contar dos veces la inactividad. Se comprueba
/// con el agregado: la suma de los `kernel` por núcleo coincide con el `kernel`
/// de `GetSystemTimes`, no con `kernel - idle`. Función pura y portable.
fn processor_times(entries: &[ProcessorPerformance]) -> Vec<(u64, u64, u64)> {
    entries
        .iter()
        .map(|entry| {
            let idle = entry.idle.max(0) as u64;
            let kernel = (entry.kernel.max(0) as u64).saturating_sub(idle);
            (entry.user.max(0) as u64, kernel, idle)
        })
        .collect()
}

/// Contadores `(usuario, kernel, idle)` de cada procesador lógico.
///
/// `GetSystemTimes` sólo da el agregado y no existe equivalente documentado
/// por núcleo. Se consulta la clase 8 de `NtQuerySystemInformation`, que el
/// SDK declara en `winternl.h` y que devuelve una entrada fija de 48 bytes por
/// procesador del grupo actual. El tamaño se pregunta primero para no asumir
/// un número máximo de núcleos.
pub fn cpu_core_times() -> io::Result<Vec<(u64, u64, u64)>> {
    const SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION: u32 = 8;
    const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004u32 as i32;
    const ENTRY_SIZE: usize = std::mem::size_of::<ProcessorPerformance>();

    let mut length = 0u32;
    // SAFETY: `length` es válido; con `information` nulo y longitud 0 la API
    // sólo informa del tamaño necesario a través de `length`.
    let status = unsafe {
        nt_query_system_information(
            SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION,
            ptr::null_mut(),
            0,
            &mut length,
        )
    };
    if status != STATUS_INFO_LENGTH_MISMATCH || length as usize % ENTRY_SIZE != 0 {
        return Err(io::Error::last_os_error());
    }

    // `u64` garantiza la alineación de 8 bytes que exigen los LARGE_INTEGER.
    let mut storage = vec![0u64; (length as usize + 7) / 8];
    let mut returned = 0u32;
    // SAFETY: el buffer es alineado y tiene exactamente el tamaño que acaba
    // de pedir la API; `returned` apila una variable válida.
    let status = unsafe {
        nt_query_system_information(
            SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION,
            storage.as_mut_ptr().cast::<c_void>(),
            length,
            &mut returned,
        )
    };
    if status != 0 || returned as usize % ENTRY_SIZE != 0 || returned as usize > storage.len() * 8 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: la API rellenó `returned / ENTRY_SIZE` registros contiguos
    // dentro del buffer alineado; se leen dentro de los límites devueltos.
    let entries = unsafe {
        std::slice::from_raw_parts(
            storage.as_ptr().cast::<ProcessorPerformance>(),
            returned as usize / ENTRY_SIZE,
        )
    };
    Ok(processor_times(entries))
}

/// Devuelve `(total, disponible)` en bytes de memoria física.
pub fn memory_bytes() -> io::Result<(u64, u64)> {
    // SAFETY: estructura POD sin punteros ni referencias; `dwLength` se fija
    // antes de la llamada, como exige la API.
    let mut status: MemoryStatusEx = unsafe { zeroed() };
    status.length = size_of::<MemoryStatusEx>() as u32;

    // SAFETY: `status` es una estructura inicializada y alineada; la API
    // escribe exactamente `dwLength` bytes.
    let ok = unsafe { global_memory_status_ex(&mut status) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((status.total_phys, status.avail_phys))
}

/// Devuelve `(total, disponible)` en bytes de `C:\`.
pub fn disk_bytes() -> io::Result<(u64, u64)> {
    let mut path: Vec<u16> = "C:\\".encode_utf16().collect();
    path.push(0);
    let mut available = 0u64;
    let mut total = 0u64;
    let mut total_free = 0u64;

    // SAFETY: la ruta termina en NUL y los tres punteros apilan variables
    // válidas durante la llamada.
    let ok = unsafe {
        get_disk_free_space_ex_w(path.as_ptr(), &mut available, &mut total, &mut total_free)
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((total, available))
}

/// Fila de `MIB_IFTABLE`, según `iprtrmib.h`.
#[repr(C)]
#[derive(Clone, Copy)]
struct MibIfRow {
    // Campos de relleno exigidos por la estructura de la API; se prefijan
    // con `_` para indicar que no se consultan.
    _name: [u16; 256],
    _index: u32,
    _kind: u32,
    _mtu: u32,
    _speed: u32,
    _phys_addr_len: u32,
    _phys_addr: [u8; 8],
    _admin_status: u32,
    _oper_status: u32,
    _last_change: u32,
    in_octets: u32,
    _in_ucast: u32,
    _in_nucast: u32,
    _in_discards: u32,
    _in_errors: u32,
    _in_unknown: u32,
    out_octets: u32,
    _out_ucast: u32,
    _out_nucast: u32,
    _out_discards: u32,
    _out_errors: u32,
    _out_qlen: u32,
    descr_len: u32,
    descr: [u8; 256],
}

#[repr(C)]
struct MibIfTable {
    entries: u32,
    rows: [MibIfRow; 1],
}

#[link(name = "iphlpapi")]
extern "system" {
    #[link_name = "GetIfTable"]
    fn get_if_table(table: *mut std::ffi::c_void, size: *mut u32, order: i32) -> u32;
}

/// Contadores RX/TX de las interfaces del sistema.
pub fn interfaces() -> io::Result<Vec<crate::collectors::network::Interface>> {
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    // Primera llamada: sólo consulta el tamaño necesario.
    let mut size = 0u32;
    // SAFETY: `size` es válido y `null` está permitido cuando el tamaño es 0.
    let code = unsafe { get_if_table(ptr::null_mut(), &mut size, 0) };
    if code != ERROR_INSUFFICIENT_BUFFER {
        return Err(io::Error::from_raw_os_error(code as i32));
    }

    // `u64` garantiza alineación de 8 bytes para la estructura devuelta.
    let mut storage = vec![0u64; (size as usize + 7) / 8];
    // SAFETY: el buffer es alineado y tiene el tamaño que acaba de pedir la API.
    let code = unsafe { get_if_table(storage.as_mut_ptr().cast::<c_void>(), &mut size, 0) };
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code as i32));
    }

    // SAFETY: la API rellenó `entries` filas contiguas de `MibIfRow` en el
    // buffer; se leen dentro de los límites devueltos por `GetIfTable`.
    let table = unsafe { &*storage.as_ptr().cast::<MibIfTable>() };
    let rows = unsafe { std::slice::from_raw_parts(table.rows.as_ptr(), table.entries as usize) };

    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let len = (row.descr_len as usize).min(row.descr.len());
        let mut name_bytes = &row.descr[..len];
        while name_bytes.last() == Some(&0) {
            name_bytes = &name_bytes[..name_bytes.len() - 1];
        }
        result.push(crate::collectors::network::Interface {
            name: String::from_utf8_lossy(name_bytes).into_owned(),
            rx_bytes: u64::from(row.in_octets),
            tx_bytes: u64::from(row.out_octets),
        });
    }
    Ok(result)
}

/// `PROCESSENTRY32W`, según `tlhelp32.h`. `th32DefaultHeapID` es `ULONG_PTR`,
/// por eso se usa `usize`: elonga 4 bytes en x86 y 8 en x64/ARM64.
#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessEntry32W {
    size: u32,
    _usage_count: u32,
    pid: u32,
    _default_heap_id: usize,
    _module_id: u32,
    threads: u32,
    _parent_pid: u32,
    _base_priority: i32,
    _flags: u32,
    exe_file: [u16; 260], // MAX_PATH
}

/// `PROCESS_MEMORY_COUNTERS`, según `psapi.h`.
#[repr(C)]
struct ProcessMemoryCounters {
    size: u32,
    _page_fault_count: u32,
    _peak_working_set_size: usize,
    working_set_size: usize,
    _quota_peak_paged_pool_usage: usize,
    _quota_paged_pool_usage: usize,
    _quota_peak_non_paged_pool_usage: usize,
    _quota_non_paged_pool_usage: usize,
    _pagefile_usage: usize,
    _peak_pagefile_usage: usize,
}

/// Devuelve el `HANDLE` inválido de forma explícita: `-1` como puntero.
fn is_invalid_handle(handle: *mut c_void) -> bool {
    handle.is_null() || handle as isize == -1
}

fn utf16_until_nul(chars: &[u16]) -> String {
    let len = chars
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..len])
}

/// Contadores de un PID: working set (bytes) y tiempo de CPU (100 ns).
///
/// Se abre un único handle para ambas consultas: antes se abría uno sólo para
/// la memoria, y abrir otro para el tiempo duplicaría las llamadas por PID y
/// por muestra. Si una consulta falla, su campo queda a cero.
fn process_counters(pid: u32) -> (u64, u64) {
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    // SAFETY: `OpenProcess` no recibe punteros de usuario; un handle inválido
    // se detecta antes de usarlo.
    let handle = unsafe { open_process(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if is_invalid_handle(handle) {
        return (0, 0);
    }

    // SAFETY: estructura POD alineada; `cb` se fija antes de la llamada.
    let mut memory: ProcessMemoryCounters = unsafe { zeroed() };
    memory.size = size_of::<ProcessMemoryCounters>() as u32;
    let mut creation = FileTime::default();
    let mut exit = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();

    // SAFETY: el handle es válido y la estructura tiene el tamaño declarado.
    let ok_memory = unsafe { get_process_memory_info(handle, &mut memory, memory.size) };
    // SAFETY: mismo handle válido; los cuatro punteros apilan `FileTime`s
    // inicializadas y la API sólo las escribe.
    let ok_times =
        unsafe { get_process_times(handle, &mut creation, &mut exit, &mut kernel, &mut user) };

    // SAFETY: el handle fue abierto por `OpenProcess` y no se guarda.
    unsafe { close_handle(handle) };

    let rss = if ok_memory == 0 {
        0
    } else {
        memory.working_set_size as u64
    };
    // `GetProcessTimes` da el tiempo acumulado de kernel y de usuario en
    // unidades de 100 ns; dos muestras permiten derivar el porcentaje.
    let cpu = if ok_times == 0 {
        0
    } else {
        kernel.as_u64().saturating_add(user.as_u64())
    };
    (rss, cpu)
}

/// Muestra intermedia de un proceso con su contador de CPU acumulado.
pub struct ProcessSample {
    pub pid: u32,
    pub name: String,
    pub threads: u32,
    pub rss_bytes: u64,
    pub cpu_100ns: u64,
}

/// Enumera los procesos con Toolhelp32 y lee sus contadores acumulados.
fn snapshot() -> io::Result<Vec<ProcessSample>> {
    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;

    // SAFETY: flags y PID son escalares; el handle se valida a continuación.
    let snapshot = unsafe { create_toolhelp32_snapshot(TH32CS_SNAPPROCESS, 0) };
    if is_invalid_handle(snapshot) {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: estructura POD alineada; `dwSize` debe fijarse antes de la
    // primera llamada, tal como exige `Process32FirstW`.
    let mut entry: ProcessEntry32W = unsafe { zeroed() };
    entry.size = size_of::<ProcessEntry32W>() as u32;

    let mut result = Vec::new();
    // SAFETY: `entry` y `snapshot` son válidos durante la llamada.
    let mut more = unsafe { process32_first_w(snapshot, &mut entry) } != 0;
    while more {
        let (rss_bytes, cpu_100ns) = process_counters(entry.pid);
        result.push(ProcessSample {
            pid: entry.pid,
            name: utf16_until_nul(&entry.exe_file),
            threads: entry.threads,
            rss_bytes,
            cpu_100ns,
        });
        // SAFETY: misma precondition que `Process32FirstW`.
        more = unsafe { process32_next_w(snapshot, &mut entry) } != 0;
    }

    // SAFETY: handle abierto por `CreateToolhelp32Snapshot` y no guardado.
    unsafe { close_handle(snapshot) };
    Ok(result)
}

/// Instantánea de procesos para calcular CPU entre dos muestras.
pub fn process_samples() -> io::Result<Vec<ProcessSample>> {
    snapshot()
}

/// Lista de procesos del sistema, sin calcular CPU.
pub fn processes() -> io::Result<Vec<crate::collectors::processes::Process>> {
    Ok(snapshot()?
        .into_iter()
        .map(|sample| crate::collectors::processes::Process {
            pid: sample.pid,
            name: sample.name,
            threads: sample.threads,
            rss_bytes: sample.rss_bytes,
            cpu_basis_points: 0,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{
        cpu_core_times, cpu_times, disk_bytes, interfaces, memory_bytes, process_samples,
        processes, processor_times, ProcessorPerformance,
    };

    #[test]
    fn kernel_por_nucleo_incluye_idle() {
        let entries = [
            ProcessorPerformance {
                idle: 40,
                kernel: 100,
                user: 60,
                _dpc: 0,
                _interrupt: 0,
                _count: 0,
                _padding: 0,
            },
            ProcessorPerformance {
                idle: 10,
                kernel: 30,
                user: 20,
                _dpc: 0,
                _interrupt: 0,
                _count: 0,
                _padding: 0,
            },
        ];
        assert_eq!(processor_times(&entries), vec![(60, 60, 40), (20, 20, 10)]);
    }

    #[test]
    fn win32_devuelve_contadores_de_cpu() {
        let (user, kernel, idle) = cpu_times().unwrap();
        assert!(user > 0 || kernel > 0, "sin tiempo de CPU acumulado");
        assert!(idle <= kernel, "idle debería estar incluido en kernel");
    }

    #[test]
    fn nucleos_son_coherentes_con_el_agregado() {
        let cores = cpu_core_times().unwrap();
        let (user, kernel, idle) = cpu_times().unwrap();
        assert!(!cores.is_empty(), "sin contadores por núcleo");

        // Las dos APIs se leen con microsegundos de diferencia: basta con una
        // tolerancia del 1 % sobre contadores acumulados de horas.
        let near = |sum: u64, total: u64| sum.abs_diff(total) * 100 <= total.max(1);
        assert!(near(cores.iter().map(|core| core.0).sum(), user));
        assert!(near(cores.iter().map(|core| core.1).sum(), kernel - idle));
        assert!(near(cores.iter().map(|core| core.2).sum(), idle));
    }

    #[test]
    fn win32_devuelve_memoria_fisica() {
        let (total, available) = memory_bytes().unwrap();
        assert!(total > 0);
        assert!(available <= total);
    }

    #[test]
    fn win32_devuelve_espacio_de_disco() {
        let (total, available) = disk_bytes().unwrap();
        assert!(total > 0);
        assert!(available <= total);
    }

    #[test]
    fn win32_devuelve_interfaces_de_red() {
        let list = interfaces().unwrap();
        for interface in list {
            assert!(!interface.name.is_empty());
        }
    }

    #[test]
    fn win32_devuelve_procesos() {
        let list = processes().unwrap();
        assert!(!list.is_empty());
        assert!(list.iter().any(|process| !process.name.is_empty()));
        assert!(list.iter().any(|process| process.rss_bytes > 0));
    }

    #[test]
    fn win32_devuelve_tiempo_acumulado_por_proceso() {
        let list = process_samples().unwrap();
        assert!(!list.is_empty());
        assert!(list.iter().any(|sample| sample.cpu_100ns > 0));
    }
}
