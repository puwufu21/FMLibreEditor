use std::collections::HashMap;
use std::ops::Range;

use super::process::Process;
use super::region::{region_at, regions};
use crate::error::Result;

/// A thread environment block (TEB) starts with an `NT_TIB`: the top of the thread's stack, the lowest address it has
/// grown to, and the block's own address. The process environment block (PEB) follows further on.
const STACK_BASE: usize = 0x08;
const STACK_LIMIT: usize = 0x10;
const SELF: usize = 0x30;
const PEB: usize = 0x60;
const READ: usize = PEB + 8;
const PAGE: u64 = 0x1000;
/// No thread of the game reserves more stack than this.
const MOST_STACK: u64 = 1 << 30;

/// The stacks of the Windows threads Wine runs in `process`, each from the bottom of the region it has grown into up
/// to its top.
///
/// Linux knows nothing of them: Wine allocates each thread's stack and its environment block in the process's own
/// memory. The blocks are found by what each says of itself: on a page of the heap, a block's `Self` holds the
/// block's own address, under it a stack top above its limit, and after it the PEB, which every thread of the process
/// shares. The one PEB most candidates name is the process's; a candidate naming another is left out. In a process
/// that Wine does not run, nothing is found.
pub fn thread_stacks(process: &Process) -> Result<Vec<Range<u64>>> {
    let pages: Vec<u64> = regions(process, 0x1_0000, 0x7FFF_FFFF_0000)
        .into_iter()
        .filter(|region| region.holds_data())
        .flat_map(|region| (region.base..region.end()).step_by(PAGE as usize))
        .collect();
    let blocks: Vec<(u64, u64, u64, u64)> = process
        .read_many(&pages, READ)
        .into_iter()
        .zip(&pages)
        .filter_map(|(bytes, &page)| teb(page, &bytes?))
        .collect();

    let mut pebs: HashMap<u64, usize> = HashMap::new();
    for &(_, _, _, peb) in &blocks {
        *pebs.entry(peb).or_default() += 1;
    }
    let Some((&peb, _)) = pebs.iter().max_by_key(|&(_, count)| *count) else {
        return Ok(Vec::new());
    };
    Ok(blocks
        .into_iter()
        .filter(|&(_, _, _, block_peb)| block_peb == peb)
        .map(|(_, top, limit, _)| {
            let bottom = region_at(process, limit).map_or(limit, |region| region.base.min(limit));
            bottom..top
        })
        .collect())
}

/// `(block, stack top, stack limit, PEB)` when the bytes read at `page` are a thread's environment block.
fn teb(page: u64, bytes: &[u8]) -> Option<(u64, u64, u64, u64)> {
    let word = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().expect("8 bytes"));
    let (top, limit, own, peb) = (word(STACK_BASE), word(STACK_LIMIT), word(SELF), word(PEB));
    let plausible = own == page
        && limit < top
        && top - limit <= MOST_STACK
        && top.is_multiple_of(16)
        && limit.is_multiple_of(16)
        && crate::is_user_pointer(peb)
        && crate::is_user_pointer(limit);
    plausible.then_some((page, top, limit, peb))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(page: u64, top: u64, limit: u64, peb: u64) -> Vec<u8> {
        let mut bytes = vec![0u8; READ];
        bytes[STACK_BASE..STACK_BASE + 8].copy_from_slice(&top.to_le_bytes());
        bytes[STACK_LIMIT..STACK_LIMIT + 8].copy_from_slice(&limit.to_le_bytes());
        bytes[SELF..SELF + 8].copy_from_slice(&page.to_le_bytes());
        bytes[PEB..PEB + 8].copy_from_slice(&peb.to_le_bytes());
        bytes
    }

    #[test]
    fn tells_an_environment_block_by_what_it_says_of_itself() {
        let page = 0x7000_0000_0000;
        let found = teb(page, &block(page, 0x6000_0010_0000, 0x6000_0000_0000, 0x7000_1000_0000));
        assert_eq!(found, Some((page, 0x6000_0010_0000, 0x6000_0000_0000, 0x7000_1000_0000)));
        // Another page's block, a stack upside down, a vast one, and a block without a PEB are not.
        assert_eq!(teb(page + PAGE, &block(page, 0x6000_0010_0000, 0x6000_0000_0000, 0x7000_1000_0000)), None);
        assert_eq!(teb(page, &block(page, 0x6000_0000_0000, 0x6000_0010_0000, 0x7000_1000_0000)), None);
        assert_eq!(teb(page, &block(page, 0x6100_0000_0000, 0x6000_0000_0000, 0x7000_1000_0000)), None);
        assert_eq!(teb(page, &block(page, 0x6000_0010_0000, 0x6000_0000_0000, 0)), None);
        assert_eq!(teb(page, &[0u8; READ]), None);
    }

    #[test]
    fn finds_a_block_laid_out_on_the_heap() {
        let process = Process::open_read(std::process::id()).expect("open this process");
        // A page-aligned block on the heap, as Wine lays one out, naming a stack also on the heap.
        let stack = vec![0u8; 0x10000];
        let limit = (stack.as_ptr() as u64).next_multiple_of(16);
        let top = limit + 0x8000;
        let mut area = vec![0u8; 3 * PAGE as usize];
        let page = (area.as_ptr() as u64).next_multiple_of(PAGE);
        let offset = (page - area.as_ptr() as u64) as usize;
        let peb = page + 0x1000;
        area[offset..offset + READ].copy_from_slice(&block(page, top, limit, peb));
        let stacks = thread_stacks(&process).expect("stacks");
        assert!(stacks.iter().any(|s| s.end == top && s.start <= limit), "{stacks:x?}");
        drop(area);
        drop(stack);
    }
}
