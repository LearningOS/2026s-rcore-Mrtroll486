//! Implementation of syscalls
//!
//! The single entry point to all system calls, [`syscall()`], is called
//! whenever userspace wishes to perform a system call using the `ecall`
//! instruction. In this case, the processor raises an 'Environment call from
//! U-mode' exception, which is handled as one of the cases in
//! [`crate::trap::trap_handler`].
//!
//! For clarity, each single syscall is implemented as its own function, named
//! `sys_` then the name of the syscall. You can find functions like this in
//! submodules, and you should also implement syscalls this way.

/// write syscall
const SYSCALL_WRITE: usize = 64;
/// exit syscall
const SYSCALL_EXIT: usize = 93;
/// yield syscall
const SYSCALL_YIELD: usize = 124;
/// gettime syscall
const SYSCALL_GET_TIME: usize = 169;
/// trace syscall
const SYSCALL_TRACE: usize = 410;
/// the number of syscalls
pub const SYSCALL_NUM: usize = 450;

mod fs;
mod process;

use fs::*;
use process::*;

use crate::task::get_cur_task_id;
use crate::loader::USER_SC_CNT;

/// handle syscall exception with `syscall_id` and other arguments
pub fn syscall(syscall_id: usize, args: [usize; 3]) -> isize {
    match syscall_id {
        SYSCALL_WRITE => { 
            unsafe {
                USER_SC_CNT[get_cur_task_id()][SYSCALL_WRITE] += 1;
            }
            sys_write(args[0], args[1] as *const u8, args[2]) 
        },
        SYSCALL_EXIT => { 
            unsafe {
                USER_SC_CNT[get_cur_task_id()][SYSCALL_EXIT] += 1;
            }
            sys_exit(args[0] as i32) 
        },
        SYSCALL_YIELD => { 
            unsafe {
                USER_SC_CNT[get_cur_task_id()][SYSCALL_YIELD] += 1;
            }
            sys_yield()
        },
        SYSCALL_GET_TIME => { 
            unsafe {
                USER_SC_CNT[get_cur_task_id()][SYSCALL_GET_TIME] += 1;
            }
            sys_get_time(args[0] as *mut TimeVal, args[1]) 
        },
        SYSCALL_TRACE => { 
            unsafe {
                USER_SC_CNT[get_cur_task_id()][SYSCALL_TRACE] += 1;
            }
            sys_trace(args[0], args[1], args[2]) 
        },
        _ => panic!("Unsupported syscall_id: {}", syscall_id),
    }
}
