use crate::{mm, x86_64_util};
use log::debug;
use spin::{Mutex, Once};
use x86::apic::xapic;

pub(crate) const IA32_APIC_BASE_MSR: u32 = 0x1B;
pub(crate) const IA32_APIC_BASE_MSR_BSP: u32 = 0x100; // Processor is a BSP
pub(crate) const IA32_APIC_BASE_MSR_ENABLE: u32 = 0x800;

const APIC_LVT_MASK_BITS: u32 = 1 << 16;

pub(crate) static XAPIC_INSTANCE: Once<Mutex<XAPIC>> = Once::new();

#[derive(Debug)]
pub struct XAPIC {
    mmio_region: &'static mut [u32],
}

impl XAPIC {
    pub fn new(address: usize) -> Self {
        let region: &'static mut [u32] = unsafe { &mut *(address as *mut [u32; 256]) };
        Self {
            mmio_region: region,
        }
    }

    /// Read a register from the MMIO region.
    pub(crate) fn read(&self, offset: u32) -> u32 {
        assert!(offset as usize % 4 == 0);
        let index = offset as usize / 4;
        unsafe { core::ptr::read_volatile(&self.mmio_region[index]) }
    }

    /// write a register in the MMIO region.
    pub(crate) fn write(&mut self, offset: u32, val: u32) {
        assert!(offset as usize % 4 == 0);
        let index = offset as usize / 4;
        unsafe { core::ptr::write_volatile(&mut self.mmio_region[index], val) }
    }
}

pub(crate) fn has_apic() -> bool {
    let value = unsafe { x86_64_util::cpuid(1) };
    value.edx & 0x100 != 0
}

pub(crate) fn init() {
    super::pic::disable_temp();

    let mut apic = XAPIC::new(mm::address::phys_to_virt(get_apic_base_address()));
    // enable apic
    set_apic_base_address(get_apic_base_address());

    let spurious = apic.read(xapic::XAPIC_SVR);
    apic.write(xapic::XAPIC_SVR, spurious | (0x100));
    let apic_id = apic.read(xapic::XAPIC_ID) >> 24;
    let apic_ver = apic.read(xapic::XAPIC_VERSION);

    debug!(
        "APIC ID:{:x}, Version:{:x}, Max LVT:{:x}",
        apic_id,
        apic_ver & 0xff,
        (apic_ver >> 16) & 0xff
    );

    debug!("spurious:{:x}", spurious);

    XAPIC_INSTANCE.call_once(|| Mutex::new(apic));
}

#[inline(always)]
pub fn ack() {
    XAPIC_INSTANCE
        .get()
        .unwrap()
        .lock()
        .write(xapic::XAPIC_EOI, 0);
}

/// set APIC base address and enable it
fn set_apic_base_address(address: usize) {
    x86_64_util::set_msr(
        IA32_APIC_BASE_MSR,
        address | IA32_APIC_BASE_MSR_ENABLE as usize,
    )
}

/// get APIC base address
fn get_apic_base_address() -> usize {
    x86_64_util::get_msr(IA32_APIC_BASE_MSR) & 0xf_ffff_f000
}