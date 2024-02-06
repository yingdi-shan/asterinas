// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use super::{
    dentry::{ExfatBitmapDentry, ExfatDentry, ExfatDentryIterator},
    fat::{ClusterID, ExfatChain},
    fs::ExfatFS,
};
use crate::{fs::exfat::fat::FatChainFlags, prelude::*, vm::vmo::Vmo};
use align_ext::AlignExt;
use aster_rights::Full;
use bitvec::prelude::*;

//TODO:use u64
type BitStore = u8;
pub(super) const EXFAT_RESERVED_CLUSTERS: u32 = 2;
const BITS_PER_BYTE: usize = 8;

#[derive(Debug, Default)]
pub(super) struct ExfatBitmap {
    // start cluster of allocation bitmap
    chain: ExfatChain,
    bitvec: BitVec<BitStore>,
    dirty_bytes: VecDeque<Range<usize>>,

    // Used to track the number of free clusters.
    num_free_cluster: u32,
    fs: Weak<ExfatFS>,
}

impl ExfatBitmap {
    pub fn load_bitmap(
        fs: Weak<ExfatFS>,
        root_page_cache: Vmo<Full>,
        root_chain: ExfatChain,
    ) -> Result<Self> {
        let dentry_iterator = ExfatDentryIterator::new(root_page_cache, 0, None)?;

        for dentry_result in dentry_iterator {
            let dentry = dentry_result?;
            if let ExfatDentry::Bitmap(bitmap_dentry) = dentry {
                // If the last bit of bitmap is 0, it is a valid bitmap.
                if (bitmap_dentry.flags & 0x1) == 0 {
                    return Self::allocate_bitmap(fs, &bitmap_dentry);
                }
            }
        }

        return_errno_with_message!(Errno::EINVAL, "bitmap not found")
    }

    fn fs(&self) -> Arc<ExfatFS> {
        self.fs.upgrade().unwrap()
    }

    fn used(&self, bit: usize) -> bool {
        *(self.bitvec.get(bit).unwrap())
    }

    fn allocate_bitmap(fs_weak: Weak<ExfatFS>, dentry: &ExfatBitmapDentry) -> Result<Self> {
        let fs = fs_weak.upgrade().unwrap();
        let num_clusters = (dentry.size as usize).align_up(fs.cluster_size()) / fs.cluster_size();

        let chain = ExfatChain::new(
            fs_weak.clone(),
            dentry.start_cluster,
            Some(num_clusters as u32),
            FatChainFlags::ALLOC_POSSIBLE,
        )?;
        let mut buf = vec![0; dentry.size as usize];

        fs.read_meta_at(chain.physical_cluster_start_offset(), &mut buf)?;
        let mut free_cluster_num = 0;
        for idx in 0..fs.super_block().num_clusters - EXFAT_RESERVED_CLUSTERS {
            if (buf[idx as usize / BITS_PER_BYTE] & (1 << (idx % BITS_PER_BYTE as u32))) == 0 {
                free_cluster_num += 1;
            }
        }
        Ok(ExfatBitmap {
            chain,
            bitvec: BitVec::from_slice(&buf),
            dirty_bytes: VecDeque::new(),
            num_free_cluster: free_cluster_num,
            fs: fs_weak,
        })
    }

    pub fn set_bitmap_used(&mut self, cluster: u32, sync: bool) -> Result<()> {
        self.set_bitmap_range(cluster..cluster + 1, true, sync)
    }

    pub fn set_bitmap_unused(&mut self, cluster: u32, sync: bool) -> Result<()> {
        self.set_bitmap_range(cluster..cluster + 1, false, sync)
    }

    pub fn set_bitmap_range_used(&mut self, clusters: Range<ClusterID>, sync: bool) -> Result<()> {
        self.set_bitmap_range(clusters, true, sync)
    }

    pub fn set_bitmap_range_unused(
        &mut self,
        clusters: Range<ClusterID>,
        sync: bool,
    ) -> Result<()> {
        self.set_bitmap_range(clusters, false, sync)
    }

    pub fn is_cluster_free(&self, cluster: u32) -> Result<bool> {
        self.is_cluster_range_free(cluster..cluster + 1)
    }

    pub fn is_cluster_range_free(&self, clusters: Range<ClusterID>) -> Result<bool> {
        if !self.fs().is_cluster_range_valid(clusters.clone()) {
            return_errno_with_message!(Errno::EINVAL, "invalid cluster ranges.")
        }

        for id in clusters {
            if self.bitvec[(id - EXFAT_RESERVED_CLUSTERS) as usize] {
                return Ok(false);
            }
        }
        Ok(true)
    }

    //Return the first free cluster
    pub fn find_next_free_cluster(&self, cluster: u32) -> Result<u32> {
        let clusters = self.find_next_free_cluster_range(cluster, 1)?;
        Ok(clusters.start)
    }

    //Return the first free cluster range, set cluster_num=1 to find a single cluster
    pub fn find_next_free_cluster_range(
        &self,
        search_start_cluster: ClusterID,
        cluster_num: u32,
    ) -> Result<Range<ClusterID>> {
        if !self
            .fs()
            .is_cluster_range_valid(search_start_cluster..search_start_cluster + cluster_num)
        {
            return_errno_with_message!(Errno::ENOSPC, "free contigous clusters not avalable.")
        }

        let mut cur_index = search_start_cluster - EXFAT_RESERVED_CLUSTERS;
        let end_index = self.fs().super_block().num_clusters - EXFAT_RESERVED_CLUSTERS;
        let search_end_index = end_index - cluster_num + 1;
        let mut range_start_index: ClusterID;

        while cur_index < search_end_index {
            if !self.used(cur_index as usize) {
                range_start_index = cur_index;
                let mut cnt = 0;
                while cnt < cluster_num && cur_index < end_index && !self.used(cur_index as usize) {
                    cnt += 1;
                    cur_index += 1;
                }
                if cnt >= cluster_num {
                    return Ok(range_start_index + EXFAT_RESERVED_CLUSTERS
                        ..range_start_index + EXFAT_RESERVED_CLUSTERS + cluster_num);
                }
            }
            cur_index += 1;
        }
        return_errno!(Errno::ENOSPC)
    }

    pub fn find_next_free_cluster_range_fast(
        &self,
        search_start_cluster: ClusterID,
        cluster_num: u32,
    ) -> Result<Range<ClusterID>> {
        if !self
            .fs()
            .is_cluster_range_valid(search_start_cluster..search_start_cluster + cluster_num)
        {
            return_errno!(Errno::ENOSPC)
        }

        let bytes: &[BitStore] = self.bitvec.as_raw_slice();
        let unit_size: u32 = (BITS_PER_BYTE * core::mem::size_of::<BitStore>()) as u32;
        let start_cluster_index = search_start_cluster - EXFAT_RESERVED_CLUSTERS;
        let mut cur_unit_index = start_cluster_index / unit_size;
        let mut cur_unit_offset = start_cluster_index % unit_size;
        let total_cluster_num = self.fs().super_block().num_clusters - EXFAT_RESERVED_CLUSTERS;
        let complete_unit_num = total_cluster_num / unit_size;
        let rest_cluster_num = total_cluster_num % unit_size;
        let mut head_cluster_num;
        let mut mid_unit_num;
        let mut tail_cluster_num;
        let mut found: bool = false;
        let mut result_bit_index = 0;
        if cluster_num > unit_size {
            // treat a continuous bit chunk as lead_bits+mid_units+tail_bits
            // mid_units are unit aligned
            // for example: 11110000 00000000 00000000 00111111
            //                  **** -------- -------- ..
            //                  ^(start bit)
            // (*): head_bits;  (-): mid_units;  (.): tail_bits
            // the start bit can be identified with a pair (cur_unit_index, cur_unit_offset)
            while cur_unit_index < complete_unit_num {
                found = true;
                head_cluster_num = unit_size - cur_unit_offset;
                mid_unit_num = (cluster_num - head_cluster_num) / unit_size;
                tail_cluster_num = (cluster_num - head_cluster_num) % unit_size;

                // if the last complete unit to be checked is out of range, stop searching
                if cur_unit_index + mid_unit_num >= complete_unit_num {
                    found = false;
                    break;
                }

                // first, check for the head bits
                let leading_zeros = bytes[cur_unit_index as usize].leading_zeros();
                if head_cluster_num > leading_zeros {
                    cur_unit_offset = unit_size - leading_zeros;
                    if cur_unit_offset == unit_size {
                        cur_unit_index += 1;
                        cur_unit_offset = 0;
                    }
                    found = false;
                    continue;
                }

                // then check for the mid units, these units should be all zero
                // due to previous check, there will be no array out of bounds situation
                for i in 1..mid_unit_num + 1 {
                    if bytes[(cur_unit_index + i) as usize] != 0 {
                        cur_unit_index += i;
                        cur_unit_offset =
                            unit_size - bytes[(cur_unit_index + i) as usize].leading_zeros();
                        if cur_unit_offset == unit_size {
                            cur_unit_index += 1;
                            cur_unit_offset = 0;
                        }
                        found = false;
                        break;
                    }
                }

                if !found {
                    continue;
                }

                // at last, check for the tail bits
                let mut tail_byte: u8 = 0;
                if cur_unit_index + mid_unit_num + 1 == complete_unit_num {
                    // for the tail part, there are two special cases:
                    //      1. this part is out of range;
                    //      2. this part exists, but are partly invaild;
                    if rest_cluster_num == 0 {
                        // in this case, the tail part is out of range
                        found = tail_cluster_num == 0;
                        result_bit_index = cur_unit_index * unit_size + cur_unit_offset;
                        break;
                    } else {
                        // in this case, the tail unit isn't a complete unit, we should set the invaild part of this unit to 1
                        // the invaild part <=> high (unit_size - rest_cluster_num) bits of tail unit
                        tail_byte |= 0xFF_u8 - ((1_u8 << rest_cluster_num) - 1);
                    }
                }
                tail_byte |= bytes[(cur_unit_index + mid_unit_num + 1) as usize];
                let tailing_zeros = tail_byte.trailing_zeros();
                if tail_cluster_num > tailing_zeros {
                    cur_unit_index = cur_unit_index + mid_unit_num + 1;
                    cur_unit_offset = tailing_zeros + 1;
                    if cur_unit_offset == unit_size {
                        cur_unit_index += 1;
                        cur_unit_offset = 0;
                    }
                    found = false;
                    continue;
                }

                // if we reach here, it means we have found a result
                result_bit_index = cur_unit_index * unit_size + cur_unit_offset;
                break;
            }
            if found {
                Ok(result_bit_index + EXFAT_RESERVED_CLUSTERS
                    ..result_bit_index + EXFAT_RESERVED_CLUSTERS + cluster_num)
            } else {
                return_errno!(Errno::ENOSPC)
            }
        } else {
            // cluster_num <= unit_size, back to the simple function
            self.find_next_free_cluster_range(search_start_cluster, cluster_num)
        }
    }

    pub fn num_free_clusters(&self) -> u32 {
        self.num_free_cluster
    }

    fn set_bitmap_range(
        &mut self,
        clusters: Range<ClusterID>,
        bit: bool,
        sync: bool,
    ) -> Result<()> {
        if !self.fs().is_cluster_range_valid(clusters.clone()) {
            return_errno_with_message!(Errno::EINVAL, "invalid cluster ranges.")
        }

        for cluster_id in clusters.clone() {
            let index = (cluster_id - EXFAT_RESERVED_CLUSTERS) as usize;
            let old_bit = self.used(index);
            self.bitvec.set(index, bit);

            if !old_bit && bit {
                self.num_free_cluster -= 1;
            } else if old_bit && !bit {
                self.num_free_cluster += 1;
            }
        }

        self.write_bitmap_range_to_disk(clusters.clone(), sync)?;

        Ok(())
    }

    fn write_bitmap_range_to_disk(&mut self, clusters: Range<ClusterID>, sync: bool) -> Result<()> {
        let unit_size = core::mem::size_of::<BitStore>() * BITS_PER_BYTE;
        let start_byte_off: usize = (clusters.start - EXFAT_RESERVED_CLUSTERS) as usize / unit_size;
        let end_byte_off: usize =
            ((clusters.end - EXFAT_RESERVED_CLUSTERS) as usize).align_up(unit_size) / unit_size;

        let bytes: &[BitStore] = self.bitvec.as_raw_slice();
        let byte_chunk = &bytes[start_byte_off..end_byte_off];

        let pos = self.chain.walk_to_cluster_at_offset(start_byte_off)?;

        let phys_offset = pos.0.physical_cluster_start_offset() + pos.1;
        self.fs().write_meta_at(phys_offset, byte_chunk)?;

        let byte_range = phys_offset..phys_offset + byte_chunk.len();

        if sync {
            self.fs().sync_meta_at(byte_range.clone())?;
        } else {
            self.dirty_bytes.push_back(byte_range.clone());
        }

        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        while let Some(range) = self.dirty_bytes.pop_front() {
            self.fs().sync_meta_at(range)?;
        }
        Ok(())
    }
}
