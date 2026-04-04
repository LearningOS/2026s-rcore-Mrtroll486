use core::usize;

use super::{
    block_cache_sync_all, get_block_cache, BlockDevice, DirEntry, DiskInode, DiskInodeType,
    EasyFileSystem, DIRENT_SZ,
};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use log::debug;
use spin::{Mutex, MutexGuard};
/// Virtual filesystem layer over easy-fs
pub struct Inode {
    block_id: usize,
    block_offset: usize,
    fs: Arc<Mutex<EasyFileSystem>>,
    block_device: Arc<dyn BlockDevice>,
    inode_id: u32,
}

impl Inode {
    /// Create a vfs inode
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        block_device: Arc<dyn BlockDevice>,
        inode_id: u32,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
            inode_id
        }
    }
    /// Call a function over a disk inode to read it
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .read(self.block_offset, f)
    }
    /// Call a function over a disk inode to modify it
    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .modify(self.block_offset, f)
    }
    /// Find inode under a disk inode by name
    fn find_inode_id(&self, name: &str, disk_inode: &DiskInode) -> Option<u32> {
        // assert it is a directory
        assert!(disk_inode.is_dir());
        let file_count = (disk_inode.size as usize) / DIRENT_SZ;
        let mut dirent = DirEntry::empty();
        for i in 0..file_count {
            assert_eq!(
                disk_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device,),
                DIRENT_SZ,
            );
            if dirent.name() == name && dirent.inode_id() != u32::MAX {
                return Some(dirent.inode_id() as u32);
            }
        }
        None
    }
    /// Find inode under current inode by name
    pub fn find(&self, name: &str) -> Option<Arc<Inode>> {
        let fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            self.find_inode_id(name, disk_inode).map(|inode_id| {
                let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
                Arc::new(Self::new(
                    block_id,
                    block_offset,
                    self.fs.clone(),
                    self.block_device.clone(),
                    inode_id
                ))
            })
        })
    }
    /// Increase the size of a disk inode
    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut MutexGuard<EasyFileSystem>,
    ) {
        if new_size < disk_inode.size {
            return;
        }
        let blocks_needed = disk_inode.blocks_num_needed(new_size);
        let mut v: Vec<u32> = Vec::new();
        for _ in 0..blocks_needed {
            v.push(fs.alloc_data());
        }
        disk_inode.increase_size(new_size, v, &self.block_device);
    }
    /// Insert a new DirEnt
    fn insert_dirent(&self, 
        name: &str, 
        new_inode_id: u32, 
        fs: &mut MutexGuard<'_, EasyFileSystem>) {
        self.modify_disk_inode(|root_inode| {
            // append file in the dirent
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let mut empty_idx = usize::MAX;
            for idx in 0..file_count {
                let mut dirent = DirEntry::empty();
                root_inode.read_at(
                idx * DIRENT_SZ,
                dirent.as_bytes_mut(),
                    &self.block_device);
                if dirent.inode_id() == u32::MAX {
                    // find a empty dirent
                    empty_idx = idx;
                    break;
                }
            }
            let dirent = DirEntry::new(name, new_inode_id);
            
            if empty_idx == usize::MAX {
                // no empty DirEnt
                let new_size = (file_count + 1) * DIRENT_SZ;
                // increase size
                self.increase_size(new_size as u32, root_inode, fs);
                // write dirent
                root_inode.write_at(
                    file_count * DIRENT_SZ,
                    dirent.as_bytes(),
                    &self.block_device,
                );
            } else {
                // found a empty DirEnt
                root_inode.write_at(
                    empty_idx * DIRENT_SZ,
                    dirent.as_bytes(),
                    &self.block_device);
            }
        });
    }
    /// Create inode under current inode by name
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        let op = |root_inode: &DiskInode| {
            // assert it is a directory
            assert!(root_inode.is_dir());
            // has the file been created?
            self.find_inode_id(name, root_inode)
        };
        if self.read_disk_inode(op).is_some() {
            return None;
        }
        // create a new file
        // alloc a inode with an indirect block
        let new_inode_id = fs.alloc_inode();
        // initialize inode
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);
        get_block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                new_inode.initialize(DiskInodeType::File);
            });
        // insert the dir file
        self.insert_dirent(name, new_inode_id, &mut fs);

        let (block_id, block_offset) = fs.get_disk_inode_pos(new_inode_id);
        block_cache_sync_all();
        // return inode
        Some(Arc::new(Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
            new_inode_id
        )))
        // release efs lock automatically by compiler
    }
    /// List inodes under current inode
    pub fn ls(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            let file_count = (disk_inode.size as usize) / DIRENT_SZ;
            let mut v: Vec<String> = Vec::new();
            for i in 0..file_count {
                let mut dirent = DirEntry::empty();
                assert_eq!(
                    disk_inode.read_at(i * DIRENT_SZ, dirent.as_bytes_mut(), &self.block_device,),
                    DIRENT_SZ,
                );
                v.push(String::from(dirent.name()));
            }
            v
        })
    }
    /// Read data from current inode
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read_at(offset, buf, &self.block_device))
    }
    /// Write data to current inode
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| {
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            disk_inode.write_at(offset, buf, &self.block_device)
        });
        block_cache_sync_all();
        size
    }
    /// Clear the data in current inode
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.modify_disk_inode(|disk_inode| {
            let size = disk_inode.size;
            let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);
            assert!(data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize);
            for data_block in data_blocks_dealloc.into_iter() {
                fs.dealloc_data(data_block);
            }
        });
        block_cache_sync_all();
    }
    /// Get inode id
    pub fn get_inode_id(&self) -> u32 {
        self.inode_id
    }
    /// return the nlink and file type in `(nlink, DiskInodeType)`
    pub fn get_stat(&self) -> (u32, DiskInodeType) {
        let fs = self.fs.lock();
        let (block_id, block_offset) = fs.get_disk_inode_pos(self.inode_id);
        get_block_cache(block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .read(block_offset, |disk_inode: & DiskInode| {
                (disk_inode.nlink, if disk_inode.is_dir() {
                    DiskInodeType::Directory}
                else {
                    DiskInodeType::File
                })
            })
    }
    /// Creat a hardlink `new_name` which links to file/hardlink `old_name`
    /// Note that this function can only be called by root dir inode
    /// Return true if success
    pub fn linkat(&self, old_name: String, new_name: String) -> bool {
        let op = |root_inode: &DiskInode| {
            // assert it is a directory
            assert!(root_inode.is_dir());
            // has the file been created?
            self.find_inode_id(&old_name, root_inode)
        };
        // return false if no file named old_name or new and old names are identical
        if self.read_disk_inode(op).is_none() || old_name == new_name {
            return false;
        }
        // increase the nlink in old file
        debug!("increase the nlink in old file");
        let old_inode_id = self.find(&old_name).unwrap().get_inode_id();
        let mut fs = self.fs.lock();
        let (old_inode_block_id, old_inode_block_offset) = fs
            .get_disk_inode_pos(old_inode_id);
        get_block_cache(
            old_inode_block_id as usize,
            Arc::clone(&self.block_device))
            .lock()
            .modify(old_inode_block_offset, |diskinode: &mut DiskInode| {
                debug!("start linking");
                diskinode.link();
            });
        // add a new DirEnt in root dir inode
        debug!("add a new DirEnt in root dir");
        self.insert_dirent(&new_name, old_inode_id, &mut fs);
        block_cache_sync_all();
        true
    }
    /// Remove a hardlink with `name`
    /// Note that this function can only be called by root dir inode
    /// Return true if success
    pub fn unlinkat(&self, name: String) -> bool {
        let op = |root_inode: &DiskInode| {
            // assert it is a directory
            assert!(root_inode.is_dir());
            // has the file been created?
            self.find_inode_id(&name, root_inode)
        };
        // return false if no file named old_name or new and old names are identical
        if self.read_disk_inode(op).is_none() {
            return false;
        }
        // decrease the nlink in old file
        debug!("increase the nlink in old file");
        let old_inode_id = self.find(&name).unwrap().get_inode_id();
        let fs = self.fs.lock();
        let (old_inode_block_id, old_inode_block_offset) = fs
            .get_disk_inode_pos(old_inode_id);
        let new_nlink = get_block_cache(
            old_inode_block_id as usize,
            Arc::clone(&self.block_device))
            .lock()
            .modify(old_inode_block_offset, |diskinode: &mut DiskInode| {
                debug!("start linking");
                diskinode.unlink()
            });
        // Remove a dirEnt with this name
        debug!("Removing a DirEnt in root dir");
        self.modify_disk_inode(|diskinode| {
            let file_cnt = (diskinode.size as usize) / DIRENT_SZ;
            for idx in 0..file_cnt {
                let mut dirent = DirEntry::empty();
                diskinode.read_at(
                    idx * DIRENT_SZ, 
                    dirent.as_bytes_mut(),
                    &self.block_device);
                if dirent.name() == &name {
                    dirent.mark_as_deleted();
                    diskinode.write_at(
                        idx * DIRENT_SZ,
                        dirent.as_bytes(),
                        &self.block_device);
                    break;
                }
            }
        });
        // if nlink is 0, then delete data
        if new_nlink == 0 {
            // the deletion should be performed
            
        }
        block_cache_sync_all();
        true
    }
}
