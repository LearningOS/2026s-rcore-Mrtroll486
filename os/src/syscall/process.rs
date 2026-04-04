//! Process management syscalls
//!
use alloc::sync::Arc;

use crate::{
    fs::{open_file, OpenFlags},
    mm::{MapPermission, translated_byte_buffer, translated_refmut, translated_str},
    task::{
        TaskControlBlock, add_task, current_task, current_user_token, exit_current_and_run_next, suspend_current_and_run_next, task_mmap, task_munmap
    },
    timer::get_time_us,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

pub fn sys_yield() -> isize {
    //trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let task = current_task().unwrap();
        task.exec(all_data.as_slice());
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    //trace!("kernel: sys_waitpid");
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// get time with second and microsecond
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_get_time",
        current_task().unwrap().pid.0
    );
    let us = get_time_us();
    let result = TimeVal{
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let slice = unsafe {
        core::slice::from_raw_parts(&result as *const TimeVal as *const u8, core::mem::size_of::<TimeVal>())
    };
    let timeval_buffer = translated_byte_buffer(current_user_token(), ts as *const u8, core::mem::size_of::<TimeVal>());
    let mut start= 0;
    for buffer in timeval_buffer{
        let len = buffer.len();
        let end = start + len;
        buffer.copy_from_slice(&slice[start..end]);
        start = end;
    }
    0
}

/// Alloc a chunk of memory starts from `start` with length `len`
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_mmap NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    if start & 0xfff != 0 || port & !0x7 != 0 || port & 0x7 == 0 {
        debug!("[kernel] mmap failed by start not aligned, invalid port");
        -1
    } else {
        // note that mmap is mapping memory for users, so set U flag
        let perm = 
            if port & 0x1 == 0x1 {MapPermission::R} else {MapPermission::empty()}
            | if port & 0x2 == 0x2 {MapPermission::W} else {MapPermission::empty()}
            | if port & 0x4 == 0x4 {MapPermission::X} else {MapPermission::empty()}
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

/// Dealloc a chunk of memory starts from `start` with length `len`
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_munmap NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
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
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// Spawn a new process, set the parent to current task
pub fn sys_spawn(path: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_spawn",
        current_task().unwrap().pid.0
    );
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(data) = open_file(path.as_str(), OpenFlags::RDONLY) {
        debug!("the inode id of appliction {} is {}", path, data.get_inode_id());
        let parent_task = current_task().unwrap();
        let child_task = Arc::new(TaskControlBlock::spawn(
            data.read_all().as_slice(), 
            Arc::downgrade(&parent_task)
        ));
        let child_pid = child_task.getpid();
        parent_task.inner_exclusive_access().children.push(child_task.clone());
        add_task(child_task);
        child_pid as isize
    } else {
        -1
    }
}

/// Change the priority of current task to `prio`
pub fn sys_set_priority(prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority",
        current_task().unwrap().pid.0
    );
    if prio >= 2 {
        let current_task = current_task().unwrap();
        current_task.inner_exclusive_access().stride.set_stride(prio as u64);
        prio
    } else {
        -1
    }
}
