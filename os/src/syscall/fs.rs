//! File and filesystem-related syscalls
use crate::fs::{OpenFlags, Stat, linkat_root, open_file, unlinkat_root};
use crate::mm::{translated_byte_buffer, translated_str, UserBuffer};
use crate::task::{current_task, current_user_token};

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_write", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return -1;
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_read", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        if !file.readable() {
            return -1;
        }
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        trace!("kernel: sys_read .. file.read");
        file.read(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_open(path: *const u8, flags: u32) -> isize {
    trace!("kernel:pid[{}] sys_open", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags).unwrap()) {
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        fd as isize
    } else {
        -1
    }
}

pub fn sys_close(fd: usize) -> isize {
    trace!("kernel:pid[{}] sys_close", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd].take();
    0
}

/// Return the status of the file with file descriptor `fd`.
/// Note that st is also a address in userspace, so don't try to directly access it
pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    trace!(
        "kernel:pid[{}] sys_fstat",
        current_task().unwrap().pid.0
    );
    let user_token = current_user_token();
    let current_task = current_task().unwrap();
    let task_fd_table 
        = &current_task.inner_exclusive_access().fd_table;
    if let Some(Some(file)) = task_fd_table.get(fd) {
        if let Some(osinode) = file.as_os_inode() {
            let (nlink, file_type) = osinode.get_stat();
            let result = Stat::new(
                osinode.get_inode_id() as u64,
                file_type,
                nlink);
            let result_slice = unsafe {
                core::slice::from_raw_parts(
                    &result as *const Stat as *const u8, 
                    core::mem::size_of::<Stat>())
            };
            let buffer = translated_byte_buffer(
                user_token,
                st as *const u8,
                core::mem::size_of::<Stat>());
            let mut start = 0;
            for buffer_section in buffer {
                let len = buffer_section.len();
                let end = start + len;
                buffer_section.copy_from_slice(&result_slice[start..end]);
                start = end;
            }
            0
        } else { -1 }
    } else { -1 }
}

/// Create a hardlink with `new_name` which links to the file `old_name`
pub fn sys_linkat(old_name: *const u8, new_name: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_linkat",
        current_task().unwrap().pid.0
    );
    let token = current_user_token();
    let old_name = translated_str(token, old_name);
    let new_name = translated_str(token, new_name);
    if old_name == new_name {
        -1
    } else {
        debug!("check pass, creating");
        if linkat_root(old_name, new_name) { 0 } else { -1 }
    }
}

/// Unlink the file `name`, if nlink reduced to 0, file deletion will be triggered
pub fn sys_unlinkat(name: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_unlinkat NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let token = current_user_token();
    let name = translated_str(token, name);
    if unlinkat_root(name) { 0 } else { -1 }
}
