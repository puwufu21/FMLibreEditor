//! `/proc/<pid>/maps`, one mapping per line: `start-end perms offset dev inode path`.

use crate::error::{Error, Result};

/// One line of `/proc/<pid>/maps`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mapping {
    pub start: u64,
    pub end: u64,
    pub read: bool,
    pub write: bool,
    pub execute: bool,
    /// `p` (private, copy on write) rather than `s` (shared).
    pub private: bool,
    pub offset: u64,
    pub inode: u64,
    /// The file mapped, or a kernel name such as `[heap]` or `[stack]`; empty for anonymous memory.
    pub path: String,
}

impl Mapping {
    /// Backed by a file on disk rather than anonymous memory or a kernel region.
    pub fn is_file(&self) -> bool {
        self.inode != 0 && self.path.starts_with('/')
    }

    /// The file's path without the ` (deleted)` the kernel adds once the file is gone.
    pub fn file_path(&self) -> &str {
        self.path.strip_suffix(" (deleted)").unwrap_or(&self.path)
    }
}

/// The mappings of `pid`, in address order.
pub(crate) fn read(pid: u32) -> Result<Vec<Mapping>> {
    let text =
        std::fs::read_to_string(format!("/proc/{pid}/maps")).map_err(|err| Error::io("read /proc/<pid>/maps", &err))?;
    Ok(text.lines().filter_map(parse).collect())
}

/// One line, or `None` when it does not read as a mapping.
pub(crate) fn parse(line: &str) -> Option<Mapping> {
    let mut fields = line.splitn(6, ' ');
    let (range, perms, offset, _dev, inode) =
        (fields.next()?, fields.next()?, fields.next()?, fields.next()?, fields.next()?);
    let path = fields.next().unwrap_or("").trim_start().to_owned();
    let (start, end) = range.split_once('-')?;
    let perms = perms.as_bytes();
    if perms.len() < 4 {
        return None;
    }
    Some(Mapping {
        start: u64::from_str_radix(start, 16).ok()?,
        end: u64::from_str_radix(end, 16).ok()?,
        read: perms[0] == b'r',
        write: perms[1] == b'w',
        execute: perms[2] == b'x',
        private: perms[3] == b'p',
        offset: u64::from_str_radix(offset, 16).ok()?,
        inode: inode.parse().ok()?,
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_line_of_each_kind() {
        let dll = parse(
            "6fffdcbf0000-6fffdcbf1000 r--p 00000000 103:02 1234567                    \
             /home/me/.local/share/Steam/steamapps/common/Football Manager 27/fm_Data/Plugins/x86_64/game_plugin.dll",
        )
        .expect("a file");
        assert_eq!((dll.start, dll.end, dll.offset), (0x6fffdcbf0000, 0x6fffdcbf1000, 0));
        assert!(dll.read && !dll.write && !dll.execute && dll.private && dll.is_file());
        assert!(dll.path.ends_with("Football Manager 27/fm_Data/Plugins/x86_64/game_plugin.dll"));

        let heap = parse("7f0000000000-7f0000100000 rw-p 00000000 00:00 0 ").expect("anonymous");
        assert!(heap.write && !heap.is_file() && heap.path.is_empty());

        let stack = parse("7ffd00000000-7ffd00021000 rw-p 00000000 00:00 0                          [stack]")
            .expect("the stack");
        assert_eq!(stack.path, "[stack]");
        assert!(!stack.is_file());

        let gone = parse("1000-2000 r-xp 00001000 08:01 42 /tmp/x.so (deleted)").expect("deleted");
        assert_eq!(gone.file_path(), "/tmp/x.so");
        assert!(parse("garbage").is_none());
    }

    #[test]
    fn reads_this_process() {
        let maps = read(std::process::id()).expect("our own maps");
        assert!(!maps.is_empty());
        let here = reads_this_process as *const () as u64;
        assert!(maps.iter().any(|m| (m.start..m.end).contains(&here) && m.execute));
    }
}
