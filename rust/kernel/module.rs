//! Modules - or to be more accurate : miscelanious
//!
//! C headers: [`include/linux/module.h`](../../../../include/linux/module.h)

use crate::alloc::allocator::Kmalloc;
use crate::c_str;
use crate::str::CStr;
use core::ffi::c_ulong;
use kernel::prelude::*;

type OptModName = Option<KVec<u8>>;
type OptSymName = Option<KVec<u8>>;

/// Lookup an address for it's associated symbol
///
/// addr : Address to lookup for
/// offset : The offset of the address in respect to the symbol
/// symbolsize : The size of the symbol (ie the space between the symbol and it's following symbol)
///
/// # Return
/// A tuple with if existing :
/// 1- the name of the module where the address is (if not the address point to kernel space)
/// 2- the name of the symbol where the address is
pub fn symbols_lookup_address(
    addr: u64,
    offset: &mut u64,
    symbolsize: &mut u64,
) -> Result<(OptModName, OptSymName)> {
    // Should call preempt_disable()
    let mut modname: *mut i8 = core::ptr::null_mut::<i8>();
    let mut ret = None;
    let mut namebuf: Vec<u8, Kmalloc> =
        Vec::from_elem(0_u8, bindings::KSYM_NAME_LEN as usize, GFP_KERNEL)?;

    let mut len = 0;
    // SAFETY: Just an FFI call
    // The return value seem to be the address of namebuf or NULL depending if a symbol has been found
    unsafe {
        if !(bindings::kallsyms_lookup(
            addr as c_ulong,
            symbolsize as *mut c_ulong,
            offset as *mut c_ulong,
            &mut modname as *mut *mut i8,
            namebuf.as_mut_ptr() as *mut i8,
        )
        .is_null())
        {
            for e in &namebuf {
                len += 1;
                if *e == 0_u8 {
                    break;
                }
            }
            // SAFETY: the size of the symbol is under the capacity of the vector
            // All the element in the vector were initialized
            // The new length is lower than the capacity and the old length
            namebuf.set_len(len);
            ret = Some(namebuf)
        }
    }

    if modname.is_null() {
        // preempt_enable();
        Ok((None, ret))
    } else {
        let mut modname_clone: Vec<u8, Kmalloc> = Vec::new();
        modname_clone.extend_from_slice(
            // SAFETY: the modname is garenteed to be valid while the process is not rescheduled
            // We are between a preempt_disable() and preempt_enable() call so it's OK
            unsafe { CStr::from_char_ptr(modname).as_bytes_with_nul() },
            GFP_KERNEL,
        )?;

        // Should call preempt_enable();
        Ok((Some(modname_clone), ret))
    }
}

/// Lookup for the symbol address
pub fn symbols_lookup_name(name: &CStr) -> u64 {
    // SAFETY: Just an FFI call
    unsafe { bindings::kallsyms_lookup_name(name.as_char_ptr()) }
}

/// Lookup for the symbol size and offset in respect to the address given
/// For the definition of these terms see the `symbol_lookup_address`'s doc
pub fn symbols_lookup_size_offset(addr: u64) -> (usize, usize) {
    let mut offset: u64 = 0;
    let mut symbolsize: u64 = 0;
    // SAFETY: Just an FFI call
    unsafe {
        bindings::kallsyms_lookup_size_offset(
            addr,
            &mut symbolsize as *mut u64,
            &mut offset as *mut u64,
        )
    };

    (symbolsize as usize, offset as usize)
}

/// Check if the name correspond to a module in the module list
pub fn is_module(name: &CStr) -> bool {
    // preempt_disable()
    // SAFETY: The call should be made with the preemption disabled
    if unsafe { bindings::find_module(name.as_char_ptr()) }.is_null() {
        return false;
    }
    // TODO call try_module_get to have a Ref see ho to integrate that with the Rust API
    // preempt_enable()
    true
}

/// Check if the address is in the kernel text (module text is not in kernel text)
/// Basically a porting of the `static inline __is_kernel`
#[cfg(target_arch = "x86_64")]
pub fn is_kernel(addr: u64) -> bool {
    let stext = symbols_lookup_name(c_str!("_stext"));
    let end = symbols_lookup_name(c_str!("_end"));
    let init_begin = symbols_lookup_name(c_str!("__init_begin"));
    let init_end = symbols_lookup_name(c_str!("__init_end"));

    // Taken from `__in_kernel` in `include/asm-generic/sections.h`
    (addr >= stext && addr < end) || (addr >= init_begin && addr < init_end)
}
