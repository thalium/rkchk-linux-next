//! INSN : In kernel decompiler
//!
//! C header : [`arch/x86/include/asm/insn.h`](../../../../arch/x86/include/asm/insn.h)

use core::{ffi::c_void, mem::MaybeUninit};
use kernel::prelude::*;

/// Represent the kernel's `struct insn` structure
/// Represent the decompiled of an instruction
pub struct Insn(bindings::insn);

fn inat_has_immediate(attr: bindings::insn_attr_t) -> u32 {
    attr & bindings::INAT_IMM_MASK
}

fn inat_immediate_size(attr: bindings::insn_attr_t) -> u32 {
    (attr & bindings::INAT_IMM_MASK) >> bindings::INAT_IMM_OFFS
}

impl Insn {
    /// Create a new `struct insn` structure and initialize it.
    /// Only one instruction is analyzed.
    /// So buffer longer than 15 bytes will only be analyzed on the 15's first bytes.
    pub fn new(buffer: &[u8]) -> Self {
        let mut insn = MaybeUninit::<bindings::insn>::uninit();

        // SAFETY: Just an FFI call.
        // The buffer len is respected.
        unsafe {
            bindings::insn_init(
                insn.as_mut_ptr(),
                buffer as *const [u8] as *const c_void,
                buffer.len() as i32,
                1,
            );
        }

        // SAFETY: According to the insn_init's API, insn is now initialized
        let insn = unsafe { insn.assume_init() };

        // insn is valid and a well initialized opcode
        Insn(insn)
    }

    /// Get the length of the parsed instruction
    pub fn get_length(&mut self) -> Result<u32> {
        // SAFETY: By the type invariant, we know that `self.0` is valid.
        let ret = unsafe { bindings::insn_get_length(&mut self.0 as _) };

        crate::error::to_result(ret)?;

        Ok(self.0.length as _)
    }

    /// Get the opcode of the parsed instruction
    pub fn get_opcode(&mut self) -> Result<i32> {
        // SAFETY: By the type invariant, we know that `self.0` is valid.
        let ret = unsafe { bindings::insn_get_opcode(&mut self.0 as _) };

        crate::error::to_result(ret)?;

        // SAFETY: By the type invariant, we know that `self.0` is valid.
        // We just initialized the opcode field and we know it didn't failed.
        let opcode = unsafe { self.0.opcode.__bindgen_anon_1.value };
        Ok(opcode)
    }

    /// Get the immediate of the parsed instruction
    pub fn get_immediate(&mut self) -> Result<Option<(u64, u8)>> {
        // SAFETY: By the type invariant, we know that `self.0` is valid.
        // So this is just an FFI call
        let ret = unsafe { bindings::insn_get_immediate(&mut self.0 as _) };

        crate::error::to_result(ret)?;

        if inat_has_immediate(self.0.attr) == 0 {
            return Ok(None);
        }

        // SAFETY: By the type invariant, we know that `self.0` is valid
        match inat_immediate_size(self.0.attr) {
            // SAFETY: By the type invariant, we know that `self.0` is valid.
            // According to the function call we access the good union field.
            bindings::INAT_IMM_BYTE | bindings::INAT_IMM_WORD | bindings::INAT_IMM_DWORD => unsafe {
                Ok(Some((
                    self.0.__bindgen_anon_1.immediate.__bindgen_anon_1.value as u32 as u64,
                    self.0.__bindgen_anon_1.immediate.nbytes,
                )))
            },
            // SAFETY: By the type invariant, we know that `self.0` is valid.
            // According to the function call we access the good union field.
            bindings::INAT_IMM_QWORD => unsafe {
                let lb: u64 =
                    self.0.__bindgen_anon_1.immediate1.__bindgen_anon_1.value as u32 as u64;
                let hb: u64 =
                    self.0.__bindgen_anon_2.immediate2.__bindgen_anon_1.value as u32 as u64;
                Ok(Some((
                    hb << 32 | lb,
                    self.0.__bindgen_anon_1.immediate.nbytes,
                )))
            },

            _ => {
                pr_err!("Unknow immediate size, skipping");
                Ok(None)
            }
        }
    }
}
