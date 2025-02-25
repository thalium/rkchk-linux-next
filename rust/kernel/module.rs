//! Modules - or to be more accurate : miscelanious
//!
//! C headers: [`include/linux/module.h`](../../../../include/linux/module.h)

use crate::alloc::allocator::Kmalloc;
use crate::str::CStr;
use crate::types::{ARef, AlwaysRefCounted, Opaque};
use crate::{c_str, container_of, transmute};
use bindings::KSYM_NAME_LEN;
use core::ffi::c_ulong;
use core::mem::transmute;
use core::ptr::addr_of;
use kernel::prelude::*;

type OptModName = Option<KVec<u8>>;
type OptSymName = Option<KVec<u8>>;

/// Get the symbol type :
/// - 'T' : exported text symbol
/// - 't' : non-exported text symbol
/// - 'D' : exported data symbol
/// - 'd' : non-exported data symbol
/// How to do it :
///     First we get the symbol index (position) in the kallsyms array : get_symbol_pos
///     Next we get the offset in the compressed stream : get_symbol_offset
///     Finally we get the type with : kallsyms_get_symbol_type, it seems like this function as of linux 6.14-rc4 has a small bug (which never trigger but anyway this is not good)
///     ^
///     |
///    This is the theory, in practice thoses symbols are either inlined or not accessible which make the process a "bit" messier  
/*fn symbol_type() -> Result<char> {

}*/

pub struct SymbolInfo {
    kallsyms_num_syms: u32,
    kallsyms_name: *const u8,
    kallsyms_token_index: *const u16,
    kallsyms_token_table: *const u8,
    kallsyms_sym_address: extern "C" fn(i32) -> u64,
}

impl SymbolInfo {
    pub fn try_new() -> Result<Self> {
        let kallsyms_name: *const u8 = symbols_lookup_name(c_str!("kallsyms_names")) as _;
        if kallsyms_name.is_null() {
            pr_err!("Couldn't find kallsyms_name symbol\n");
            return Err(EFAULT);
        }

        let kallsyms_token_index: *const u16 =
            symbols_lookup_name(c_str!("kallsyms_token_index")) as _;
        if kallsyms_token_index.is_null() {
            pr_err!("Couldn't find kallsyms_token_index symbol\n");
            return Err(EFAULT);
        }

        let kallsyms_token_table: *const u8 =
            symbols_lookup_name(c_str!("kallsyms_token_table")) as _;
        if kallsyms_token_table.is_null() {
            pr_err!("Couldn't find kallsyms_token_table symbol\n");
            return Err(EFAULT);
        }

        let pkallsyms_num_syms: *const u32 = symbols_lookup_name(c_str!("kallsyms_num_syms")) as _;
        if pkallsyms_num_syms.is_null() {
            pr_err!("Couldn't find pkallsyms_num_syms symbol\n");
            return Err(EFAULT);
        }
        let kallsyms_num_syms: u32 = unsafe { *pkallsyms_num_syms };

        let pkallsyms_sym_address: *const () =
            symbols_lookup_name(c_str!("kallsyms_sym_address")) as _;
        if pkallsyms_sym_address.is_null() {
            pr_err!("Couldn't find kallsyms_sym_address symbol\n");
            return Err(EFAULT);
        }

        let kallsyms_sym_address = unsafe { transmute(pkallsyms_sym_address) };

        Ok(SymbolInfo {
            kallsyms_num_syms,
            kallsyms_name,
            kallsyms_token_index,
            kallsyms_token_table,
            kallsyms_sym_address,
        })
    }

    /// Does pretty much the same thing as `kallsyms_expand_symbol()` expect it copy also the type information as I need it
    fn expand_symbols(
        &self,
        mut off: usize,
        buffer: &mut [u8; KSYM_NAME_LEN as _],
    ) -> Result<usize> {
        let mut data: *const u8 = self.kallsyms_name.wrapping_add(off);
        let mut len: usize = unsafe { *data } as _;

        data = data.wrapping_add(1);
        off += 1;

        // If the MSB of len is not null, this is a big symbol and the len is stored on 2 bytes
        if len & 0x80 != 0 {
            len = (len & 0x7F) | ((unsafe { *data } as usize) << 7);
            data = data.wrapping_add(1);
            off += 1;
        }

        // We update the offset with the len of the symbol to return the offset of the next symbol
        off += len;

        let mut i: usize = 0;

        'outer: while len != 0 {
            // We get a pointer to a token and we copy the token to the buffer, that way we decompress the symbol
            let token_index = unsafe { *self.kallsyms_token_index.wrapping_add(*data as _) };
            let mut ptoken = self.kallsyms_token_table.wrapping_add(token_index as _);

            while unsafe { *ptoken } != 0 {
                if let Some(r) = buffer.get_mut(i) {
                    *r = unsafe { *ptoken };
                } else {
                    break 'outer;
                }
                ptoken = ptoken.wrapping_add(1);
                i += 1;
            }

            len -= 1;
            data = data.wrapping_add(1);
        }

        let buffer_len = buffer.len();

        if let Some(r) = buffer.get_mut(i) {
            *r = 0;
        } else if let Some(r) = buffer.get_mut(buffer_len - 1) {
            *r = 0;
        }

        return Ok(off);
    }

    pub fn on_each(&self, f: impl Fn(&[u8; KSYM_NAME_LEN as _], u64) -> Result<()>) -> Result<()> {
        let mut off = 0;
        let mut buffer = KBox::new([0_u8; KSYM_NAME_LEN as _], GFP_KERNEL)?;
        for i in 0..self.kallsyms_num_syms {
            off = self.expand_symbols(off, &mut buffer)?;

            let address = unsafe { (self.kallsyms_sym_address)(i as _) };

            f(&buffer, address);
        }
        Ok(())
    }
}

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

/// Check if the address is in the kernel's module space
/// Using the
#[repr(transparent)]
pub struct Module {
    inner: Opaque<bindings::module>,
}

impl Module {
    /// Create a rust Module structure from a raw ptr to a C `struct module`
    ///
    /// # Safety
    ///
    /// The raw pointer should be valid, non-null, and have a non-zero refcount
    /// i.e. it must ensure that the reference count of the C `struct module`
    /// can't drop to zero, for the duration of this function call.    
    pub unsafe fn get_module(module: *mut bindings::module) -> ARef<Self> {
        // SAFETY: `bindings::module` and `Module` have the same memory representation
        unsafe { &*(module as *const Module) }.into()
    }

    /// Return a raw pointer to the inner structure
    pub const fn as_ptr(&self) -> *mut bindings::module {
        self.inner.get()
    }

    /// Print the name of the module
    pub fn print_name(&self) {
        let ptr = self.inner.get();

        // SAFETY: ptr is non null, point to valid data and is aligned
        // according to the type invariant and the C guarantees
        let name_bytes = unsafe { addr_of!((*ptr).name) as *const i8 };

        // SAFETY: `module.name` is valid for the lifetime of the module,
        // we hold a refcount of the module so it is valid for the duration of this call
        // and we have that name is not null and is a constant null terminated string
        let name = unsafe { CStr::from_char_ptr(name_bytes) };

        pr_info!("Module : {:?}\n", name);
    }
}

// SAFETY: The type invariant guarantte that Module is always refcounted.
// By the kernel API, while the refcount is not 0 the object is alive
unsafe impl AlwaysRefCounted for Module {
    unsafe fn dec_ref(obj: core::ptr::NonNull<Self>) {
        // SAFETY: The safety requirement guarantee that the refcount is non null
        unsafe { bindings::module_put(obj.as_ref().as_ptr()) }
    }

    // There is sadly no way to indicate that the inc_ref might fail
    // We hope it never happen for the moment
    fn inc_ref(&self) {
        // SAFETY: The existence of a shared reference means that the refcount is not 0
        if !unsafe { bindings::try_module_get(self.as_ptr()) } {
            pr_alert!("Couldn't get the module!\n");
        }
    }
}

impl From<&ThisModule> for ARef<Module> {
    fn from(value: &ThisModule) -> Self {
        // SAFETY: The function will be executed in the context of ThisModule so the
        // refcount cannot be null, and the raw pointer is non-null and valid
        let module = unsafe { Module::get_module(value.as_ptr()) };

        // We increment the reference count of the module now that we own a refcounted copy
        module.inc_ref();

        module
    }
}

/// Iterator on all the module in the module linked list
pub struct ModuleIter {
    cur: Option<ARef<Module>>,
    head: *mut bindings::list_head,
}

impl ModuleIter {
    /// Create a new instance, this is safe because we can only read
    pub fn new() -> Self {
        let head = symbols_lookup_name(c_str!("modules")) as *mut bindings::list_head;
        ModuleIter { cur: None, head }
    }
}

impl Default for ModuleIter {
    fn default() -> Self {
        Self::new()
    }
}

impl Iterator for ModuleIter {
    type Item = ARef<Module>;

    fn next(&mut self) -> Option<Self::Item> {
        let next = match self.cur {
            // SAFETY: We have by the C API that `head.next` is valid
            None => unsafe { &*self.head }.next,
            // SAFETY: We have by the C API that `list.next` is valid
            // actually we need to hold module_mutex to guaranty this but holding
            // a C mutex from the Rust side is not supported for now)
            Some(ref module) => unsafe { (*module.as_ptr()).list }.next,
        };

        if next == self.head {
            None
        } else {
            // SAFETY: We are on the module's linked list so excepting the head they are all in `module` struct
            let next_mod = unsafe { container_of!(next, bindings::module, list) as *mut _ };
            // SAFETY: We can always call this function
            let next_mod = unsafe { Module::get_module(next_mod) };
            next_mod.inc_ref();
            self.cur = Some(next_mod.clone());
            Some(next_mod)
        }
    }
}
