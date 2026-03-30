//! Process management syscalls
use core::{mem};

use crate::mm::{MapPermission, PageTable, VirtAddr, translated_byte_buffer};
use crate::task::{change_program_brk, current_user_token, exit_current_and_run_next, get_current_syscall_cnt, suspend_current_and_run_next, task_mmap, task_munmap};
use crate::timer::{get_time_us};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// Done: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let result = TimeVal{
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let slice = unsafe {
        core::slice::from_raw_parts(&result as *const TimeVal as *const u8, mem::size_of::<TimeVal>())
    };
    let timeval_buffer = translated_byte_buffer(current_user_token(), ts as *const u8, mem::size_of::<TimeVal>());
    let mut start= 0;
    for buffer in timeval_buffer{
        let len = buffer.len();
        let end = start + len;
        buffer.copy_from_slice(&slice[start..end]);
        start = end;
    }
    0
}

/// Get the count of calling syscall with id `id`, or read/write into the address `id`
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    match trace_request {
        0 | 1 => {
            // 0: read address id and return
            // 1: write address id with data, return 0
            let usr_pagetable = PageTable::from_token(current_user_token());
            let vaddr: VirtAddr = id.into();
            match usr_pagetable.translate(vaddr.floor()) {
                Some(pte) => {
                    debug!("found pte for trace read/write");
                    if !pte.is_valid() || !pte.usr_accessable() {
                        -1
                    } else {
                        let addr = id & 0xfff | pte.ppn().0 << 12;
                        if trace_request == 0 {
                            if !pte.readable() {
                                -1
                            } else {
                                unsafe {
                                    *(addr as *const u8) as isize
                                }                            
                            }
                        } else {
                            if !pte.writable() {
                                -1
                            } else {
                                unsafe {
                                    *(addr as *mut u8) = data as u8;
                                }
                                0
                            }
                        }
                    }
                },
                None => { -1 }
            }
        },
        2 => {
            // get the count of calling syscall id
            get_current_syscall_cnt(id) as isize
        },
        _ => -1
    }
}

/// map a chunk of memory which starts from `start` with length `len` and with prmission `prot`
/// Note that `start` should align with page size (4KiB).
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap");
    if start & 0xfff != 0 || prot & !0x7 != 0 || prot & 0x7 == 0 {
        debug!("[kernel] mmap failed by start not aligned, invalid prot");
        -1
    } else {
        // note that mmap is mapping memory for users, so set U flag
        let perm = 
            if prot & 0x1 == 0x1 {MapPermission::R} else {MapPermission::empty()}
            | if prot & 0x2 == 0x2 {MapPermission::W} else {MapPermission::empty()}
            | if prot & 0x4 == 0x4 {MapPermission::X} else {MapPermission::empty()}
            | MapPermission::U;
        match task_mmap(start, len, perm) {
            0 => 0,
            1 => {
                debug!("[kernel] mmap failed by already mapped pages in [start, start + len)");
                -1
            },
            2 => {
                debug!("[kernel] mmap failed by out of memory");
                -1
            },
            _ => { -1 }
        }
    }
}

/// unmap a chunk of memory which starts from `start` with length `len`
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap ");
    if start & 0xfff != 0 {
        debug!("[kernel] munmap failed by start not aligned");
        -1
    } else {
        match task_munmap(start, len) {
            0 => 0,
            1 => {
                debug!("[kernel] unmapped pages in [start, start + len]");
                -1
            },
            2 => {
                debug!("[kernel] no map_areas are removed (no MapArea with range [start, start + len)");
                -1
            },
            _ => { -1 }
        }
    }
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
