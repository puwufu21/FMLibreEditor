use std::path::PathBuf;
use std::sync::Mutex;

use super::maps;
use super::region::{Region, region_at};
use crate::error::{Error, Result};
use crate::module::Module;

/// A process seen in `/proc`; not opened.
#[derive(Debug, Clone)]
pub struct ProcessEntry {
    pub pid: u32,
    pub exe_name: String,
}

/// Lists running processes whose executable name equals `exe_name`, ignoring case.
///
/// Under Wine the kernel's name for the game's process (`comm`) is the Windows executable's, cut to 15 bytes, while
/// the executable the kernel ran is Wine's loader. The first word of the command line is the Windows path, such as
/// `Z:\home\me\...\fm.exe` or `C:\...\fm.exe`, whose last part is checked too.
pub fn find_processes(exe_name: &str) -> Result<Vec<ProcessEntry>> {
    let dir = std::fs::read_dir("/proc").map_err(|err| Error::io("read /proc", &err))?;
    let mut found = Vec::new();
    for entry in dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
        let comm = comm.trim_end_matches('\n');
        let argv0 = std::fs::read(entry.path().join("cmdline"))
            .ok()
            .and_then(|cmdline| cmdline.split(|&b| b == 0).next().map(|arg| String::from_utf8_lossy(arg).into_owned()))
            .unwrap_or_default();
        let argv0_name = argv0.rsplit(['\\', '/']).next().unwrap_or("");
        if argv0_name.eq_ignore_ascii_case(exe_name) {
            found.push(ProcessEntry { pid, exe_name: argv0_name.to_owned() });
        } else if comm.eq_ignore_ascii_case(exe_name) && !argv0.is_empty() {
            found.push(ProcessEntry { pid, exe_name: comm.to_owned() });
        }
    }
    found.sort_by_key(|entry| entry.pid);
    Ok(found)
}

/// Whether this process runs as root.
pub fn is_current_process_elevated() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// A process whose memory is read, and written only when opened with [`Process::open_read_write`]. Linux needs no
/// handle: each call names the process by its id, and the kernel checks the editor's right to it each time.
pub struct Process {
    pid: u32,
    writable: bool,
    /// The PE modules last listed, which tell a module's statics from the heap.
    modules: Mutex<Option<Vec<Module>>>,
}

impl Process {
    pub fn open_read(pid: u32) -> Result<Self> {
        Self::open(pid, false)
    }

    /// Opens `pid` for reading and also for writing its memory with [`Process::write_bytes`].
    pub fn open_read_write(pid: u32) -> Result<Self> {
        Self::open(pid, true)
    }

    /// Checks at once that the process exists and that the kernel lets the editor read it, so that a refusal is said
    /// when connecting rather than as a failed read later.
    fn open(pid: u32, writable: bool) -> Result<Self> {
        let process = Self { pid, writable, modules: Mutex::new(None) };
        let maps = maps::read(pid)?;
        if let Some(first) = maps.iter().find(|m| m.read && !m.path.starts_with("[v")) {
            let mut byte = [0u8; 1];
            if let Err(Error::Read { code, .. }) = process.read_into(first.start, &mut byte) {
                return Err(Error::Posix { op: "process_vm_readv", code: code as i32 });
            }
        }
        Ok(process)
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Whether the process was opened for writing.
    pub fn can_write(&self) -> bool {
        self.writable
    }

    /// Full path of the Windows executable as a Linux path: the file Wine mapped for the `.exe` the command line names,
    /// so that the install it is in (`steamapps/common/...`) can be told.
    pub fn image_path(&self) -> Result<PathBuf> {
        let cmdline =
            std::fs::read(format!("/proc/{}/cmdline", self.pid)).map_err(|err| Error::io("read cmdline", &err))?;
        let argv0 =
            cmdline.split(|&b| b == 0).next().map(|arg| String::from_utf8_lossy(arg).into_owned()).unwrap_or_default();
        let name = argv0.rsplit(['\\', '/']).next().unwrap_or("").to_owned();
        if !name.is_empty() {
            let maps = maps::read(self.pid)?;
            if let Some(mapped) = maps.iter().find(|m| {
                m.is_file()
                    && m.offset == 0
                    && m.file_path().rsplit('/').next().is_some_and(|f| f.eq_ignore_ascii_case(&name))
            }) {
                return Ok(PathBuf::from(mapped.file_path()));
            }
        }
        // Wine's drive Z: is the Linux root.
        if let Some(rest) = argv0.strip_prefix("Z:").or_else(|| argv0.strip_prefix("z:")) {
            return Ok(PathBuf::from(rest.replace('\\', "/")));
        }
        std::fs::read_link(format!("/proc/{}/exe", self.pid)).map_err(|err| Error::io("read /proc/<pid>/exe", &err))
    }

    /// Only Windows packages (Game Pass) have one; a game under Wine never does.
    pub fn package_full_name(&self) -> Result<Option<String>> {
        Ok(None)
    }

    /// Fills `buf` from target memory at `addr`; fails unless every byte could be read.
    pub fn read_into(&self, addr: u64, buf: &mut [u8]) -> Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        let local = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
        let remote = libc::iovec { iov_base: addr as *mut libc::c_void, iov_len: buf.len() };
        let read = unsafe { libc::process_vm_readv(self.pid as libc::pid_t, &local, 1, &remote, 1, 0) };
        if read != buf.len() as isize {
            let code = if read < 0 { errno() } else { libc::EFAULT as u32 };
            return Err(Error::Read { addr, len: buf.len(), code });
        }
        Ok(())
    }

    /// Reads `len` bytes at each of `addrs` in as few calls as the kernel allows, for many small reads spread over
    /// memory. A place that cannot be read is `None`.
    pub(crate) fn read_many(&self, addrs: &[u64], len: usize) -> Vec<Option<Vec<u8>>> {
        const BATCH: usize = 512;
        let mut out = Vec::with_capacity(addrs.len());
        for batch in addrs.chunks(BATCH) {
            let mut buf = vec![0u8; batch.len() * len];
            let local: Vec<libc::iovec> = buf
                .chunks_mut(len)
                .map(|chunk| libc::iovec { iov_base: chunk.as_mut_ptr().cast(), iov_len: len })
                .collect();
            let remote: Vec<libc::iovec> =
                batch.iter().map(|&addr| libc::iovec { iov_base: addr as *mut libc::c_void, iov_len: len }).collect();
            let read = unsafe {
                libc::process_vm_readv(
                    self.pid as libc::pid_t,
                    local.as_ptr(),
                    local.len() as libc::c_ulong,
                    remote.as_ptr(),
                    remote.len() as libc::c_ulong,
                    0,
                )
            };
            if read == (batch.len() * len) as isize {
                out.extend(buf.chunks(len).map(|chunk| Some(chunk.to_vec())));
            } else {
                // Part of the batch failed: read each place on its own.
                out.extend(batch.iter().map(|&addr| self.read_bytes(addr, len).ok()));
            }
        }
        out
    }

    pub fn read_bytes(&self, addr: u64, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.read_into(addr, &mut buf)?;
        Ok(buf)
    }

    pub fn read_u64(&self, addr: u64) -> Result<u64> {
        let mut b = [0u8; 8];
        self.read_into(addr, &mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    pub fn read_u32(&self, addr: u64) -> Result<u32> {
        let mut b = [0u8; 4];
        self.read_into(addr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    /// Takes `len` bytes of new memory in the process: private, readable and writable, zeroed.
    ///
    /// Only this process can be given memory from outside it on Linux: another process would have to be made to ask
    /// for it itself (a call injected through ptrace), which this editor does not do. In the game, what needs new
    /// memory (a longer stadium name, a list with no room for one more) therefore fails with `ENOSYS`, and changes
    /// nothing.
    pub fn allocate(&self, len: usize) -> Result<u64> {
        if !self.writable {
            return Err(Error::ReadOnly);
        }
        if self.pid != std::process::id() {
            return Err(Error::Posix { op: "allocate in the game", code: libc::ENOSYS });
        }
        let at = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if at == libc::MAP_FAILED {
            return Err(Error::Posix { op: "mmap", code: errno() as i32 });
        }
        Ok(at as u64)
    }

    /// Only heap data can be written: the whole range must lie in one region of private, writable and non-executable
    /// memory outside every module. Anything else, such as a module's code or data, is refused.
    pub fn write_bytes(&self, addr: u64, data: &[u8]) -> Result<()> {
        self.write_where(addr, data, Region::holds_data)
    }

    /// Writes `data` at `addr` among a module's statics: the whole range must lie in one region of a PE module's image
    /// that is writable and not executable. Its code, and what it only reads, are refused.
    pub fn write_static(&self, addr: u64, data: &[u8]) -> Result<()> {
        self.write_where(addr, data, Region::holds_statics)
    }

    fn write_where(&self, addr: u64, data: &[u8], allowed: fn(&Region) -> bool) -> Result<()> {
        if !self.writable {
            return Err(Error::ReadOnly);
        }
        if data.is_empty() {
            return Ok(());
        }
        let len = data.len();
        let end = addr.checked_add(len as u64).ok_or(Error::NotWritable { addr, len })?;
        match region_at(self, addr) {
            Some(region) if allowed(&region) && end <= region.end() => {}
            _ => return Err(Error::NotWritable { addr, len }),
        }
        let local = libc::iovec { iov_base: data.as_ptr() as *mut libc::c_void, iov_len: len };
        let remote = libc::iovec { iov_base: addr as *mut libc::c_void, iov_len: len };
        let written = unsafe { libc::process_vm_writev(self.pid as libc::pid_t, &local, 1, &remote, 1, 0) };
        if written != len as isize {
            let code = if written < 0 { errno() } else { libc::EFAULT as u32 };
            return Err(Error::Write { addr, len, code });
        }
        Ok(())
    }

    pub(crate) fn remember_modules(&self, modules: &[Module]) {
        if let Ok(mut known) = self.modules.lock() {
            *known = Some(modules.to_vec());
        }
    }

    pub(crate) fn known_modules(&self) -> Option<Vec<Module>> {
        self.modules.lock().ok()?.clone()
    }
}

pub(crate) fn errno() -> u32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_heap_data_only() {
        let process = Process::open_read_write(std::process::id()).expect("open this process");
        let heap = vec![0u8; 64].into_boxed_slice();
        let addr = heap.as_ptr() as u64;

        process.write_bytes(addr + 8, &[1, 2, 3]).expect("write into the heap");
        assert_eq!(process.read_bytes(addr + 6, 6).expect("read back"), [0, 0, 1, 2, 3, 0]);

        let code = writes_heap_data_only as *const () as u64;
        assert!(matches!(process.write_bytes(code, &[0xCC]), Err(Error::NotWritable { .. })));
        assert!(matches!(process.write_bytes(0x10, &[1]), Err(Error::NotWritable { .. })));
        // A native process has no PE module, so it has no statics the editor may write.
        assert!(matches!(process.write_static(addr, &[1]), Err(Error::NotWritable { .. })));

        let read_only = Process::open_read(std::process::id()).expect("open this process");
        assert!(matches!(read_only.write_bytes(addr, &[1]), Err(Error::ReadOnly)));
        drop(heap);
    }

    #[test]
    fn reads_many_places_at_once() {
        let process = Process::open_read(std::process::id()).expect("open this process");
        let values: Vec<u64> = (0..2000).collect();
        let addrs: Vec<u64> = values.iter().map(|v| v as *const u64 as u64).chain([0x10]).collect();
        let read = process.read_many(&addrs, 8);
        assert_eq!(read.len(), 2001);
        for (i, value) in read[..2000].iter().enumerate() {
            assert_eq!(value.as_deref(), Some(&(i as u64).to_le_bytes()[..]));
        }
        assert!(read[2000].is_none());
    }

    #[test]
    fn allocates_writable_memory_only_when_open_for_writing() {
        let process = Process::open_read_write(std::process::id()).expect("open this process");
        let at = process.allocate(0x1000).expect("allocate");
        assert_eq!(process.read_bytes(at, 4).expect("read it"), [0, 0, 0, 0]);
        process.write_bytes(at + 16, b"FMLE").expect("write into it");
        assert_eq!(process.read_bytes(at + 16, 4).expect("read back"), *b"FMLE");
        let read_only = Process::open_read(std::process::id()).expect("open this process");
        assert!(matches!(read_only.allocate(0x1000), Err(Error::ReadOnly)));
    }

    #[test]
    fn a_missing_process_does_not_open() {
        assert!(Process::open_read(u32::MAX - 1).is_err());
    }
}
