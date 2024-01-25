use super::{Inode, InodeMode, InodeType};
use crate::device::Random;
use crate::prelude::*;
use alloc::sync::Arc;
use hashbrown::HashMap;

pub struct FileInMemory {
    pub name: String,
    pub inode: Arc<dyn Inode>,
    pub valid_len: usize,
    pub contents: Vec<u8>,
}
pub struct DirInMemory {
    pub depth: u32,
    pub name: String,
    pub inode: Arc<dyn Inode>,
    pub sub_names: Vec<String>,
    pub sub_dirs: HashMap<String, DentryInMemory>,
}
pub enum DentryInMemory {
    File(FileInMemory),
    Dir(DirInMemory),
}
pub enum Operation {
    Read(usize, usize),
    Write(usize, usize),
    Resize(usize),
    Create(String, InodeType),
    Lookup(String),
    Readdir(),
    Unlink(String),
    Rmdir(String),
    Rename(String, String),
}

impl DirInMemory {
    pub fn remove_sub_names(&mut self, name: &String) {
        for idx in 0..self.sub_names.len() {
            if self.sub_names[idx].eq(name) {
                self.sub_names.remove(idx);
                break;
            }
        }
    }
    pub fn execute_and_test(&mut self, op: Operation) {
        match op {
            Operation::Create(name, type_) => {
                info!(
                    "Create: parent = {:?}, name = {:?}, type = {:?}",
                    self.name, name, type_
                );

                let create_result = self.inode.create(&name, type_, InodeMode::all());
                if self.sub_dirs.contains_key(&name) {
                    assert!(create_result.is_err());
                    info!(
                        "    create {:?}/{:?} failed: {:?}",
                        self.name,
                        name,
                        create_result.unwrap_err()
                    );

                    return;
                }

                assert!(
                    create_result.is_ok(),
                    "Fail to create {:?}: {:?}",
                    name,
                    create_result.unwrap_err()
                );

                info!(
                    "    create {:?}/{:?}({:?}) succeeeded",
                    self.name, name, type_
                );

                let new_dentry_in_mem = if type_ == InodeType::File {
                    let file = FileInMemory {
                        name: name.clone(),
                        inode: create_result.unwrap(),
                        valid_len: 0,
                        contents: Vec::<u8>::new(),
                    };
                    DentryInMemory::File(file)
                } else {
                    DentryInMemory::Dir(DirInMemory {
                        depth: self.depth + 1,
                        name: name.clone(),
                        inode: create_result.unwrap(),
                        sub_names: Vec::new(),
                        sub_dirs: HashMap::new(),
                    })
                };
                let _ = self.sub_dirs.insert(name.to_string(), new_dentry_in_mem);
                self.sub_names.push(name.to_string());
            }
            Operation::Lookup(name) => {
                info!("Lookup: parent = {:?}, name = {:?}", self.name, name);

                let lookup_result = self.inode.lookup(&name);
                if self.sub_dirs.get(&name).is_some() {
                    assert!(
                        lookup_result.is_ok(),
                        "Fail to lookup {:?}: {:?}",
                        name,
                        lookup_result.unwrap_err()
                    );

                    info!("    lookup {:?}/{:?} succeeded", self.name, name);
                } else {
                    assert!(lookup_result.is_err());
                    info!(
                        "    lookup {:?}/{:?} failed: {:?}",
                        self.name,
                        name,
                        lookup_result.unwrap_err()
                    );
                }
            }
            Operation::Readdir() => {
                info!("Readdir: parent = {:?}", self.name);

                let mut sub: Vec<String> = Vec::new();
                let readdir_result = self.inode.readdir_at(0, &mut sub);
                assert!(readdir_result.is_ok(), "Fail to read directory",);
                assert!(readdir_result.unwrap() == self.sub_dirs.len() + 2);
                assert!(sub.len() == self.sub_dirs.len() + 2);

                sub.remove(0);
                sub.remove(0);

                sub.sort();
                self.sub_names.sort();

                for (i, name) in sub.iter().enumerate() {
                    assert!(
                        name.eq(&self.sub_names[i]),
                        "Directory entry mismatch: read {:?} should be {:?}",
                        name,
                        self.sub_names[i]
                    );
                }
            }
            Operation::Unlink(name) => {
                info!("Unlink: parent = {:?}, name = {:?}", self.name, name);

                let unlink_result = self.inode.unlink(&name);
                if let Option::Some(sub) = self.sub_dirs.get(&name)
                    && let DentryInMemory::File(_) = sub
                {
                    assert!(
                        unlink_result.is_ok(),
                        "Fail to remove file {:?}/{:?}: {:?}",
                        self.name,
                        name,
                        unlink_result.unwrap_err()
                    );

                    info!("    unlink {:?}/{:?} succeeded", self.name, name);

                    let _ = self.sub_dirs.remove(&name);
                    self.remove_sub_names(&name);
                } else {
                    assert!(unlink_result.is_err());

                    info!(
                        "    unlink {:?}/{:?} failed: {:?}",
                        self.name,
                        name,
                        unlink_result.unwrap_err()
                    );
                }
            }
            Operation::Rmdir(name) => {
                info!("Rmdir: parent = {:?}, name = {:?}", self.name, name);

                let rmdir_result = self.inode.rmdir(&name);
                if let Option::Some(sub) = self.sub_dirs.get(&name)
                    && let DentryInMemory::Dir(sub_dir) = sub
                    && sub_dir.sub_dirs.is_empty()
                {
                    assert!(
                        rmdir_result.is_ok(),
                        "Fail to remove directory {:?}/{:?}: {:?}",
                        self.name,
                        name,
                        rmdir_result.unwrap_err()
                    );

                    info!("    rmdir {:?}/{:?} succeeded", self.name, name);

                    let _ = self.sub_dirs.remove(&name);
                    self.remove_sub_names(&name);
                } else {
                    assert!(rmdir_result.is_err());

                    info!(
                        "    rmdir {:?}/{:?} failed: {:?}",
                        self.name,
                        name,
                        rmdir_result.unwrap_err()
                    );
                }
            }
            Operation::Rename(old_name, new_name) => {
                info!(
                    "Rename: parent = {:?}, old_name = {:?}, target = {:?}, new_name = {:?}",
                    self.name, old_name, self.name, new_name
                );

                let rename_result = self.inode.rename(&old_name, &self.inode, &new_name);

                if old_name.eq(&new_name) {
                    assert!(rename_result.is_ok());

                    info!(
                        "    rename {:?}/{:?} to {:?}/{:?} succeeded",
                        self.name, old_name, self.name, new_name
                    );

                    return;
                }

                let mut valid_rename: bool = false;
                let mut exist: bool = false;
                if let Option::Some(old_sub) = self.sub_dirs.get(&old_name) {
                    let exist_new_sub = self.sub_dirs.get(&new_name);
                    match old_sub {
                        DentryInMemory::File(old_file) => {
                            if let Option::Some(exist_new_sub_) = exist_new_sub
                                && let DentryInMemory::File(exist_new_file) = exist_new_sub_
                            {
                                valid_rename = true;
                                exist = true;
                            } else if exist_new_sub.is_none() {
                                valid_rename = true;
                            }
                        }
                        DentryInMemory::Dir(old_dir) => {
                            if let Option::Some(exist_new_sub_) = exist_new_sub
                                && let DentryInMemory::Dir(exist_new_dir) = exist_new_sub_
                                && exist_new_dir.sub_dirs.is_empty()
                            {
                                valid_rename = true;
                                exist = true;
                            } else if exist_new_sub.is_none() {
                                valid_rename = true;
                            }
                        }
                    }
                }

                if valid_rename {
                    assert!(
                        rename_result.is_ok(),
                        "Fail to rename {:?}/{:?} to {:?}/{:?}: {:?}",
                        self.name,
                        old_name,
                        self.name,
                        new_name,
                        rename_result.unwrap_err()
                    );

                    info!(
                        "    rename {:?}/{:?} to {:?}/{:?} succeeded",
                        self.name, old_name, self.name, new_name
                    );

                    let lookup_new_inode_result = self.inode.lookup(&new_name);
                    assert!(
                        lookup_new_inode_result.is_ok(),
                        "Fail to lookup new name {:?}: {:?}",
                        new_name,
                        lookup_new_inode_result.unwrap_err()
                    );
                    let mut old = self.sub_dirs.remove(&old_name).unwrap();
                    self.remove_sub_names(&old_name);
                    match old {
                        DentryInMemory::Dir(ref mut dir) => {
                            dir.inode = lookup_new_inode_result.unwrap();
                            dir.name = new_name.clone();
                            dir.depth = self.depth + 1;
                        }
                        DentryInMemory::File(ref mut file) => {
                            file.inode = lookup_new_inode_result.unwrap();
                            file.name = new_name.clone();
                        }
                    }
                    if exist {
                        let _ = self.sub_dirs.remove(&new_name);
                        self.remove_sub_names(&new_name);
                    }
                    self.sub_dirs.insert(new_name.to_string(), old);
                    self.sub_names.push(new_name.to_string());
                } else {
                    assert!(rename_result.is_err());

                    info!(
                        "    rename {:?}/{:?} to {:?}/{:?} failed: {:?}",
                        self.name,
                        old_name,
                        self.name,
                        new_name,
                        rename_result.unwrap_err()
                    );
                }
            }
            _ => {}
        }
    }
}

impl FileInMemory {
    pub fn execute_and_test(&mut self, op: Operation) {
        match op {
            Operation::Read(offset, len) => {
                info!(
                    "Read: name = {:?}, offset = {:?}, len = {:?}",
                    self.name, offset, len
                );

                let mut buf = vec![0; len];

                let read_result = self.inode.read_at(offset, &mut buf);
                assert!(
                    read_result.is_ok(),
                    "Fail to read file in range [{:?}, {:?}): {:?}",
                    offset,
                    offset + len,
                    read_result.unwrap_err()
                );

                info!("    read succeeded");

                let (start, end) = (
                    offset.min(self.valid_len),
                    (offset + len).min(self.valid_len),
                );
                assert!(
                    buf[..(end - start)].eq(&self.contents[start..end]),
                    "Read file contents mismatch"
                )
            }
            Operation::Write(offset, len) => {
                // Avoid holes in a file
                let (write_start_offset, write_len) = if offset > self.valid_len {
                    (self.valid_len, len + offset - self.valid_len)
                } else {
                    (offset, len)
                };

                info!(
                    "Write: name = {:?}, offset = {:?}, len = {:?}",
                    self.name, write_start_offset, write_len
                );

                let mut buf = vec![0; write_len];

                let _ = Random::getrandom(&mut buf);
                let write_result = self.inode.write_at(write_start_offset, &buf);
                assert!(
                    write_result.is_ok(),
                    "Fail to write file in range [{:?}, {:?}): {:?}",
                    write_start_offset,
                    write_start_offset + write_len,
                    write_result.unwrap_err()
                );

                info!("    write succeeded");

                if write_start_offset + write_len > self.contents.len() {
                    self.contents.resize(write_start_offset + write_len, 0);
                }
                self.valid_len = self.valid_len.max(write_start_offset + write_len);

                self.contents[write_start_offset..write_start_offset + write_len]
                    .copy_from_slice(&buf[..write_len]);
            }
            Operation::Resize(new_size) => {
                info!("Resize: name = {:?}, new_size = {:?}", self.name, new_size);

                // todo: may need more consideration
                let resize_result = self.inode.resize(new_size);
                assert!(
                    resize_result.is_ok(),
                    "Fail to resize file to {:?}: {:?}",
                    new_size,
                    resize_result.unwrap_err()
                );
                self.contents.resize(new_size, 0);
                self.valid_len = self.valid_len.min(new_size);
            }
            _ => {}
        }
    }
}

impl DentryInMemory {
    pub fn execute_and_test(&mut self, op: Operation) {
        match self {
            DentryInMemory::Dir(dir) => {
                dir.execute_and_test(op);
            }
            DentryInMemory::File(file) => {
                file.execute_and_test(op);
            }
        }
    }

    pub fn sub_cnt(&self) -> usize {
        match self {
            DentryInMemory::Dir(dir) => dir.sub_names.len(),
            DentryInMemory::File(file) => 0,
        }
    }
}

// generate a random number in range [0, max]
fn get_random_in(max: usize) -> usize {
    const MAX_RANDOM: usize = 0xFF;
    let mut random: [u8; 1] = [0];
    let _ = Random::getrandom(&mut random);
    let rand = random[0] as usize;
    (rand * (max + 1) / MAX_RANDOM).min(max)
}

fn random_select_from_dir_tree(root: &mut DentryInMemory) -> &mut DentryInMemory {
    let sub_cnt = root.sub_cnt();
    if sub_cnt == 0 {
        root
    } else {
        let stop_get_deeper = get_random_in(1) > 0;
        if stop_get_deeper {
            root
        } else if let DentryInMemory::Dir(dir) = root {
            let sub_idx = get_random_in(sub_cnt - 1);
            let sub = dir.sub_dirs.get_mut(&dir.sub_names[sub_idx]);
            let sub_dir = sub.unwrap();
            return random_select_from_dir_tree(sub_dir);
        } else {
            unreachable!();
        }
    }
}

fn generate_random_offset_len(max_size: usize) -> (usize, usize) {
    let offset = get_random_in(max_size - 1);
    let len = get_random_in(max_size - offset);
    (offset, len)
}

pub fn new_fs_in_memory(root: Arc<dyn Inode>) -> DentryInMemory {
    DentryInMemory::Dir(DirInMemory {
        depth: 0,
        name: (&"root").to_string(),
        inode: root,
        sub_names: Vec::new(),
        sub_dirs: HashMap::new(),
    })
}

pub fn generate_random_operation(
    root: &mut DentryInMemory,
    idx: u32,
) -> (&mut DentryInMemory, Operation) {
    const CREATE_FILE_ID: usize = 0;
    const CREATE_DIR_ID: usize = 1;
    const UNLINK_ID: usize = 2;
    const RMDIR_ID: usize = 3;
    const LOOKUP_ID: usize = 4;
    const READDIR_ID: usize = 5;
    const RENAME_ID: usize = 6;
    const DIR_OP_NUM: usize = 7;
    const READ_ID: usize = 0;
    const WRITE_ID: usize = 1;
    const RESIZE_ID: usize = 2;
    const FILE_OP_NUM: usize = 3;
    const MAX_PAGE_PER_FILE: usize = 10;
    let dentry = random_select_from_dir_tree(root);
    match dentry {
        DentryInMemory::Dir(dir) => {
            let op_id = get_random_in(DIR_OP_NUM - 1);
            if op_id == CREATE_FILE_ID {
                (dentry, Operation::Create(idx.to_string(), InodeType::File))
            } else if op_id == CREATE_DIR_ID {
                return (dentry, Operation::Create(idx.to_string(), InodeType::Dir));
            } else if op_id == UNLINK_ID && !dir.sub_names.is_empty() {
                let rand_idx = get_random_in(dir.sub_names.len() - 1);
                let name = dir.sub_names[rand_idx].clone();
                return (dentry, Operation::Unlink(name));
            } else if op_id == RMDIR_ID && !dir.sub_names.is_empty() {
                let rand_idx = get_random_in(dir.sub_names.len() - 1);
                let name = dir.sub_names[rand_idx].clone();
                return (dentry, Operation::Rmdir(name));
            } else if op_id == LOOKUP_ID && !dir.sub_names.is_empty() {
                let rand_idx = get_random_in(dir.sub_names.len() - 1);
                let name = dir.sub_names[rand_idx].clone();
                return (dentry, Operation::Lookup(name));
            } else if op_id == READDIR_ID {
                return (dentry, Operation::Readdir());
            } else if op_id == RENAME_ID && !dir.sub_names.is_empty() {
                let rand_old_idx = get_random_in(dir.sub_names.len() - 1);
                let old_name = dir.sub_names[rand_old_idx].clone();
                let rename_to_an_exist = get_random_in(1) > 0;
                if rename_to_an_exist {
                    let rand_new_idx = get_random_in(dir.sub_names.len() - 1);
                    let new_name = dir.sub_names[rand_new_idx].clone();
                    return (dentry, Operation::Rename(old_name, new_name));
                } else {
                    return (dentry, Operation::Rename(old_name, idx.to_string()));
                }
            } else {
                return (dentry, Operation::Create(idx.to_string(), InodeType::File));
            }
        }
        DentryInMemory::File(file) => {
            let op_id = get_random_in(FILE_OP_NUM - 1);
            if op_id == READ_ID {
                let (offset, len) = generate_random_offset_len(MAX_PAGE_PER_FILE * PAGE_SIZE);
                (dentry, Operation::Read(offset, len))
            } else if op_id == WRITE_ID {
                let (offset, len) = generate_random_offset_len(MAX_PAGE_PER_FILE * PAGE_SIZE);
                return (dentry, Operation::Write(offset, len));
            } else if op_id == RESIZE_ID {
                let pg_num = get_random_in(MAX_PAGE_PER_FILE);
                let new_size = (pg_num * PAGE_SIZE).max(file.contents.len());
                return (dentry, Operation::Resize(new_size));
            } else {
                let valid_len = file.valid_len;
                return (dentry, Operation::Read(0, valid_len));
            }
        }
    }
}
