//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the whole operating system.
//!
//! A single global instance of [`Processor`] called `PROCESSOR` monitors running
//! task(s) for each core.
//!
//! A single global instance of `PID_ALLOCATOR` allocates pid for user apps.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.
mod context;
mod id;
mod manager;
mod processor;
mod switch;
#[allow(clippy::module_inception)]
#[allow(rustdoc::private_intra_doc_links)]
mod task;

use crate::fs::{open_file, OpenFlags};
use crate::mm::{MapPermission, VirtAddr, VPNRange};
use alloc::sync::Arc;
pub use context::TaskContext;
use lazy_static::*;
pub use manager::{fetch_task, TaskManager};
use switch::__switch;
pub use task::{TaskControlBlock, TaskStatus};

pub use id::{kstack_alloc, pid_alloc, KernelStack, PidHandle};
pub use manager::add_task;
pub use processor::{
    current_task, current_trap_cx, current_user_token, run_tasks, schedule, take_current_task,
    Processor,
};
/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    // There must be an application running.
    let task = take_current_task().unwrap();

    // ---- access current TCB exclusively
    let mut task_inner = task.inner_exclusive_access();
    let task_cx_ptr = &mut task_inner.task_cx as *mut TaskContext;
    // Change status to Ready
    task_inner.task_status = TaskStatus::Ready;
    drop(task_inner);
    // ---- release current PCB

    // push back to ready queue.
    add_task(task);
    // jump to scheduling cycle
    schedule(task_cx_ptr);
}

/// pid of usertests app in make run TEST=1
pub const IDLE_PID: usize = 0;

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next(exit_code: i32) {
    // take from Processor
    let task = take_current_task().unwrap();

    let pid = task.getpid();
    if pid == IDLE_PID {
        println!(
            "[kernel] Idle process exit with exit_code {} ...",
            exit_code
        );
        panic!("All applications completed!");
    }

    // **** access current TCB exclusively
    let mut inner = task.inner_exclusive_access();
    // Change status to Zombie
    inner.task_status = TaskStatus::Zombie;
    // Record exit code
    inner.exit_code = exit_code;
    // do not move to its parent but under initproc

    // ++++++ access initproc TCB exclusively
    {
        let mut initproc_inner = INITPROC.inner_exclusive_access();
        for child in inner.children.iter() {
            child.inner_exclusive_access().parent = Some(Arc::downgrade(&INITPROC));
            initproc_inner.children.push(child.clone());
        }
    }
    // ++++++ release parent PCB

    inner.children.clear();
    // deallocate user space
    inner.memory_set.recycle_data_pages();
    // drop file descriptors
    inner.fd_table.clear();
    drop(inner);
    // **** release current PCB
    // drop task manually to maintain rc correctly
    drop(task);
    // we do not have to save task context
    let mut _unused = TaskContext::zero_init();
    schedule(&mut _unused as *mut _);
}

lazy_static! {
    /// Creation of initial process
    ///
    /// the name "initproc" may be changed to any other app name like "usertests",
    /// but we have user_shell, so we don't need to change it.
    pub static ref INITPROC: Arc<TaskControlBlock> = Arc::new({
        let inode = open_file("ch6b_initproc", OpenFlags::RDONLY).unwrap();
        let v = inode.read_all();
        TaskControlBlock::new(v.as_slice())
    });
}

///Add init process to the manager
pub fn add_initproc() {
    add_task(INITPROC.clone());
}

/// alloc a chunk of memory for current task, starts from `start`, with length `len`, and with 
/// permission `perm`. Returns 0 if success, 1 if already mapped pages in [start, start + len)
/// 2 if Out of memory
pub fn task_mmap(start: usize, len: usize, perm: MapPermission) -> usize {
    let inner = current_task().unwrap();
    let cur_task_id = inner.pid.0;
    let mut inner = inner.inner_exclusive_access();
    let cur_task_memset = &mut inner.memory_set;

    let start_vaddr= VirtAddr::from(start);
    let end_vaddr = VirtAddr::from(start + len);
    debug!("[kernel] mapping area: [{:#x}, {:#x}) for pid {}", start_vaddr.floor().0, end_vaddr.ceil().0, cur_task_id);
    for vpn in VPNRange::new(start_vaddr.floor(), end_vaddr.ceil()) {
        debug!("checkling page {:#x} for pid {}", start_vaddr.floor().0, cur_task_id);
        if let Some(pte) = cur_task_memset.translate(vpn) {
            if pte.is_valid() {
                debug!("mmap failed because already mapped");
                return 1;
            }
        }
    }

    cur_task_memset.insert_framed_area(start_vaddr, end_vaddr, perm);
    0
}

/// dealloc a chunk of memory for current task, starts from `start`, with length `len`.
/// Returns 0 if success, 1 if unmapped pages in [start, start + len)
/// 2 if no map_area is removed (no identical start and end)
pub fn task_munmap(start: usize, len: usize) -> usize {
    let inner = current_task().unwrap();
    let cur_task_id = inner.pid.0;
    let mut inner = inner.inner_exclusive_access();
    let cur_task_memset = &mut inner.memory_set;
    
    let start_vaddr = VirtAddr::from(start);
    let end_vaddr = VirtAddr::from(start + len);
    debug!("[kernel] unmapping area: start {:#x}, length {:#x} for pid {}", start, len, cur_task_id);
    for vpn in VPNRange::new(start_vaddr.floor(), end_vaddr.ceil()) {
        if let Some(pte) = cur_task_memset.translate(vpn) {
            if !pte.is_valid() {
                debug!("munmap faileed because page {:#x} is not mapped", vpn.0);
                return 1
            }
        } else {
            debug!("munmap faileed because page {:#x} is not mapped", vpn.0);
            return 1
        }
    }

    if cur_task_memset.unmap(start_vaddr, end_vaddr) { 0 } else { 2 }
}
