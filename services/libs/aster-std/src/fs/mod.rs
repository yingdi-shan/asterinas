// SPDX-License-Identifier: MPL-2.0
pub mod device;
pub mod devpts;
pub mod epoll;
pub mod exfat;
pub mod ext2;
pub mod file_handle;
pub mod file_table;
pub mod fs_resolver;
pub mod inode_handle;
pub mod pipe;
pub mod procfs;
pub mod ramfs;
pub mod rootfs;
pub mod utils;

use crate::fs::exfat::{ExfatFS, ExfatMountOptions};
use crate::fs::{ext2::Ext2, fs_resolver::FsPath};
use crate::prelude::*;
use crate::thread::kernel_thread::KernelThreadExt;
use alloc::format;
use aster_block::BlockDevice;
use aster_virtio::device::block::device::BlockDevice as VirtIoBlockDevice;
use aster_virtio::device::block::DEVICE_NAME as VIRTIO_BLOCK_NAME;

fn start_block_device(device_name: &str) -> Arc<dyn BlockDevice> {
    let device = aster_block::get_device(device_name).unwrap();
    let cloned_device = device.clone();
    let task_fn = move || {
        info!("spawn the virt-io-block thread");
        let virtio_block_device = cloned_device.downcast_ref::<VirtIoBlockDevice>().unwrap();
        loop {
            virtio_block_device.handle_requests();
        }
    };
    crate::Thread::spawn_kernel_thread(crate::ThreadOptions::new(task_fn));
    device
}

pub fn lazy_init() {
    let ext2_device_name = format!("{}{}", VIRTIO_BLOCK_NAME, "6");
    let exfat_device_name = format!("{}{}", VIRTIO_BLOCK_NAME, "7");

    let block_device_ext2 = start_block_device(&ext2_device_name);
    let block_device_exfat = start_block_device(&exfat_device_name);

    let ext2_fs = Ext2::open(block_device_ext2).unwrap();
    let target_path = FsPath::try_from("/ext2").unwrap();
    println!("[kernel] Mount Ext2 fs at {:?} ", target_path);
    self::rootfs::mount_fs_at(ext2_fs, &target_path).unwrap();

    let exfat_fs = ExfatFS::open(block_device_exfat, ExfatMountOptions::default()).unwrap();
    let target_path = FsPath::try_from("/exfat").unwrap();
    println!("[kernel] Mount ExFat fs at {:?} ", target_path);
    self::rootfs::mount_fs_at(exfat_fs, &target_path).unwrap();
}
