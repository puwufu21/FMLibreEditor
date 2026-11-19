use super::maps::{self, Mapping};
use super::module;
use super::process::Process;
use crate::module::Module;

/// What a region of memory holds, as far as the editor cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Part of a PE module Wine loaded: its headers, code or statics, whether Wine mapped them from the file or copied
    /// them into anonymous memory.
    Image,
    /// Anonymous memory private to the process: the heaps of Wine and of the game.
    Private,
    /// The stack of the process's first thread, as the kernel set it up.
    Stack,
    /// A file mapped outside the PE modules, shared memory, or a kernel region such as `[vdso]`.
    Mapped,
}

/// A run of pages with the same protection, from one line of `/proc/<pid>/maps`, split where a PE module starts or
/// ends (the kernel merges neighbouring anonymous mappings that a module and the heap after it may both be).
#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub base: u64,
    pub size: u64,
    /// Linux keeps no allocation a region was carved from: this is the region's own base.
    pub allocation_base: u64,
    pub read: bool,
    pub write: bool,
    pub execute: bool,
    pub kind: Kind,
}

impl Region {
    pub fn end(&self) -> u64 {
        self.base + self.size
    }

    /// Every mapped region is backed; reserved address space Wine has not committed is mapped without access.
    pub fn is_committed(&self) -> bool {
        true
    }

    pub fn is_image(&self) -> bool {
        self.kind == Kind::Image
    }

    /// Memory the process allocated for itself (heaps), as opposed to a mapped file or module.
    pub fn is_private(&self) -> bool {
        self.kind == Kind::Private
    }

    /// Whether the editor may write here: heap data, writable and not executable.
    pub fn holds_data(&self) -> bool {
        self.is_private() && self.is_writable() && !self.is_executable()
    }

    /// Whether the editor may write one of a module's statics here: memory of a module's image that the process
    /// writes and does not run, its data and not its code.
    pub fn holds_statics(&self) -> bool {
        self.is_image() && self.is_writable() && !self.is_executable()
    }

    pub fn is_readable(&self) -> bool {
        self.read
    }

    pub fn is_writable(&self) -> bool {
        self.read && self.write
    }

    pub fn is_executable(&self) -> bool {
        self.read && self.execute
    }

    /// Protection as a short "RWX" string, e.g. "RW-" or "R-X".
    pub fn protection(&self) -> String {
        if !self.is_readable() {
            return "---".to_owned();
        }
        let w = if self.is_writable() { 'W' } else { '-' };
        let x = if self.is_executable() { 'X' } else { '-' };
        format!("R{w}{x}")
    }
}

/// The regions of `process` that lie in `[start, end)`, or overlap it, in address order. Unlike Windows, Linux lists
/// only what is mapped, so the gaps between them are simply left out.
pub fn regions(process: &Process, start: u64, end: u64) -> Vec<Region> {
    let Ok(maps) = maps::read(process.pid()) else {
        return Vec::new();
    };
    let modules = module::from_maps(process, &maps);
    process.remember_modules(&modules);
    maps.iter()
        .filter(|m| m.end > start && m.start < end)
        .flat_map(|m| split(m, &modules))
        .filter(|r| r.end() > start && r.base < end)
        .collect()
}

/// The region that contains `addr`, or `None` where nothing is mapped.
pub fn region_at(process: &Process, addr: u64) -> Option<Region> {
    let maps = maps::read(process.pid()).ok()?;
    let mapping = maps.iter().find(|m| (m.start..m.end).contains(&addr))?;
    let modules = process.known_modules().unwrap_or_else(|| {
        let modules = module::from_maps(process, &maps);
        process.remember_modules(&modules);
        modules
    });
    split(mapping, &modules).into_iter().find(|r| (r.base..r.end()).contains(&addr))
}

/// `mapping` as one region, or several where PE modules start or end inside it.
fn split(mapping: &Mapping, modules: &[Module]) -> Vec<Region> {
    let mut cuts = vec![mapping.start, mapping.end];
    for module in modules {
        for edge in [module.base, module.end()] {
            if edge > mapping.start && edge < mapping.end {
                cuts.push(edge);
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .map(|pair| {
            let in_module = modules.iter().any(|module| module.contains(pair[0]));
            Region {
                base: pair[0],
                size: pair[1] - pair[0],
                allocation_base: pair[0],
                read: mapping.read,
                write: mapping.write,
                execute: mapping.execute,
                kind: kind_of(mapping, in_module),
            }
        })
        .collect()
}

fn kind_of(mapping: &Mapping, in_module: bool) -> Kind {
    if in_module {
        Kind::Image
    } else if mapping.path == "[stack]" {
        Kind::Stack
    } else if mapping.private
        && (mapping.path.is_empty() || mapping.path == "[heap]" || mapping.path.starts_with("[anon"))
    {
        Kind::Private
    } else {
        Kind::Mapped
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn mapping(line: &str) -> Mapping {
        maps::parse(line).expect("a mapping")
    }

    #[test]
    fn a_module_inside_an_anonymous_mapping_is_cut_out_of_it() {
        // Wine copied the module's sections into anonymous memory, which the kernel merged with the heap after it.
        let merged = mapping("10000-50000 rw-p 00000000 00:00 0 ");
        let module = Module { name: "game_plugin.dll".into(), base: 0x8000, size: 0x20000, path: PathBuf::new() };
        let regions = split(&merged, &[module]);
        assert_eq!(regions.len(), 2);
        assert_eq!((regions[0].base, regions[0].end(), regions[0].kind), (0x10000, 0x28000, Kind::Image));
        assert_eq!((regions[1].base, regions[1].end(), regions[1].kind), (0x28000, 0x50000, Kind::Private));
        assert!(regions[0].holds_statics() && !regions[0].holds_data());
        assert!(regions[1].holds_data() && !regions[1].holds_statics());
    }

    #[test]
    fn what_each_mapping_holds() {
        let none: [Module; 0] = [];
        let kind = |line| split(&mapping(line), &none)[0].kind;
        assert_eq!(kind("1000-2000 rw-p 00000000 00:00 0 "), Kind::Private);
        assert_eq!(kind("1000-2000 rw-p 00000000 00:00 0 [heap]"), Kind::Private);
        assert_eq!(kind("1000-2000 rw-p 00000000 00:00 0 [stack]"), Kind::Stack);
        assert_eq!(kind("1000-2000 rw-s 00000000 00:01 5 /dev/shm/wine"), Kind::Mapped);
        assert_eq!(kind("1000-2000 rw-p 00002000 08:01 42 /usr/lib/libc.so.6"), Kind::Mapped);
        assert_eq!(kind("1000-2000 r-xp 00000000 00:00 0 [vdso]"), Kind::Mapped);
        let code = split(&mapping("1000-2000 r-xp 00000000 00:00 0 "), &none)[0];
        assert!(code.is_executable() && !code.holds_data() && code.protection() == "R-X");
        let reserved = split(&mapping("1000-2000 ---p 00000000 00:00 0 "), &none)[0];
        assert!(!reserved.is_readable() && reserved.protection() == "---");
    }

    #[test]
    fn regions_of_this_process_cover_its_heap_and_code() {
        let process = Process::open_read(std::process::id()).expect("open this process");
        let heap = vec![0u8; 1 << 20];
        let at = heap.as_ptr() as u64;
        let region = region_at(&process, at).expect("the heap is mapped");
        assert!(region.holds_data(), "{region:?}");
        let code = region_at(&process, regions_of_this_process_cover_its_heap_and_code as *const () as u64)
            .expect("the code is mapped");
        assert!(code.is_executable() && !code.holds_data());
        let all = regions(&process, 0x1_0000, 0x7FFF_FFFF_0000);
        assert!(all.iter().any(|r| (r.base..r.end()).contains(&at)));
        assert!(all.windows(2).all(|pair| pair[0].end() <= pair[1].base));
        assert!(region_at(&process, 0x10).is_none());
    }
}
