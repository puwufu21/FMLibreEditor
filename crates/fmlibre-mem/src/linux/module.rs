use std::path::{Path, PathBuf};

use super::maps::{self, Mapping};
use super::pe::PeImage;
use super::process::Process;
use crate::error::Result;
use crate::module::Module;

/// Lists the PE modules (EXEs and DLLs) Wine loaded into `process`.
///
/// Wine maps a module's file from its start, at least the page of its headers; how much more depends on the file:
/// when the sections are not aligned to pages, as in `game_plugin.dll`, it copies them into anonymous memory and only
/// the headers' page stays a mapping of the file. A module is therefore told by the mapping of a `.dll` or `.exe`
/// from its start, and sized by the `SizeOfImage` of the headers read there, not by what `/proc` shows of the file.
pub fn list_modules(process: &Process) -> Result<Vec<Module>> {
    let modules = from_maps(process, &maps::read(process.pid())?);
    process.remember_modules(&modules);
    Ok(modules)
}

pub(crate) fn from_maps(process: &Process, maps: &[Mapping]) -> Vec<Module> {
    let mut modules: Vec<Module> = Vec::new();
    for mapping in maps.iter().filter(|m| m.is_file() && m.offset == 0 && m.read && is_pe_file(m.file_path())) {
        if modules.iter().any(|module| module.contains(mapping.start)) {
            continue;
        }
        let Ok(image) = PeImage::read(process, mapping.start) else {
            continue;
        };
        let path = PathBuf::from(mapping.file_path());
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        modules.push(Module { name, base: mapping.start, size: u64::from(image.size_of_image), path });
    }
    modules
}

fn is_pe_file(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("dll") || ext.eq_ignore_ascii_case("exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dlls_and_exes_are_modules() {
        assert!(is_pe_file("/games/Football Manager 27/fm_Data/Plugins/x86_64/game_plugin.dll"));
        assert!(is_pe_file("/games/Football Manager 27/FM.EXE"));
        assert!(!is_pe_file("/usr/lib/libc.so.6"));
        assert!(!is_pe_file("/games/Football Manager 27/fm_Data/data.unity3d"));
    }

    /// The headers of a PE32+ image of `size_of_image` bytes with no section, as a file holds them.
    fn pe_headers(size_of_image: u32) -> Vec<u8> {
        let mut page = vec![0u8; 0x1000];
        page[..2].copy_from_slice(b"MZ");
        page[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        page[0x80..0x84].copy_from_slice(b"PE\0\0");
        let optional = 0x80 + 4 + 20;
        page[0x80 + 4 + 16..0x80 + 4 + 18].copy_from_slice(&240u16.to_le_bytes());
        page[optional..optional + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
        page[optional + 56..optional + 60].copy_from_slice(&size_of_image.to_le_bytes());
        page
    }

    #[test]
    fn a_dll_mapped_from_its_start_is_a_module_of_its_declared_size() {
        let dir = std::env::temp_dir().join(format!("fmlibre-mem-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let path = dir.join("game_plugin.dll");
        std::fs::write(&path, pe_headers(0x5000)).expect("the file");
        let file = std::fs::File::open(&path).expect("open it");
        let at = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                0x1000,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                std::os::fd::AsRawFd::as_raw_fd(&file),
                0,
            )
        };
        assert_ne!(at, libc::MAP_FAILED);

        let process = Process::open_read(std::process::id()).expect("open this process");
        let modules = list_modules(&process).expect("modules");
        let module = modules.iter().find(|m| m.base == at as u64).expect("the DLL is a module");
        assert_eq!((module.name.as_str(), module.size), ("game_plugin.dll", 0x5000));
        assert_eq!(module.path, path);
        let region = crate::linux::region::region_at(&process, at as u64).expect("mapped");
        assert!(region.is_image());

        unsafe { libc::munmap(at, 0x1000) };
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_native_process_has_no_pe_modules() {
        let process = Process::open_read(std::process::id()).expect("open this process");
        // Only the DLL another test may have mapped meanwhile.
        let temp = std::env::temp_dir();
        assert!(list_modules(&process).expect("modules").iter().all(|module| module.path.starts_with(&temp)));
    }
}
