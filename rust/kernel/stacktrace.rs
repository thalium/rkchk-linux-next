//! Stacktrace : stack trace getter
//!
//! C header : [`arch/x86/include/stacktrace.h`](../../../../include/linux/stacktrace.h)

use core::ops::Deref;

use crate::{
    alloc::{Flags, KVec},
    kernel::error::Result,
};

/// Represent a captured stacktrace of the current process
pub struct Stacktrace(KVec<u64>);

impl Stacktrace {
    /// Save a new stacktrace of the current process
    pub fn new(size: usize, flag: Flags) -> Result<Self> {
        let mut buf = KVec::from_elem(0u64, size, flag)?;

        // SAFETY: This function save the stacktrace of the current process so it is always safe to call
        let len = unsafe { bindings::stack_trace_save(buf.as_mut_ptr(), size as _, 0) };

        // SAFETY: We have by the `stack_trace_save` contract that `len<size` so `new_len<old_len`.
        unsafe { buf.set_len(len as _) };
        Ok(Stacktrace(buf))
    }
}

impl Deref for Stacktrace {
    type Target = KVec<u64>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
