//! Linux: the Windows game running under Wine or Proton. The kernel sees one process, `fm.exe`, whose memory holds
//! the game's PE modules as Wine loaded them. Processes and memory maps come from `/proc`, memory through
//! `process_vm_readv` and `process_vm_writev`, and modules from the PE headers Wine mapped.
//!
//! The kernel lets one process into another's memory under the same rules as a debugger (ptrace): with the default
//! `kernel.yama.ptrace_scope = 1` only a parent may, unless the editor has `CAP_SYS_PTRACE`. The editor changes no
//! such setting; it reports the refusal.

mod maps;
mod module;
#[path = "../windows/pe.rs"]
mod pe;
mod process;
mod region;
mod thread;

pub use module::list_modules;
pub use pe::{Export, PeImage, Section};
pub use process::{Process, ProcessEntry, find_processes, is_current_process_elevated};
pub use region::{Region, region_at, regions};
pub use thread::thread_stacks;
