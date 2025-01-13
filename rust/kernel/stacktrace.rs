//! Stacktrace : stack trace getter
//!
//! C header : [`arch/x86/include/stacktrace.h`](../../../../include/linux/stacktrace.h)

use crate::{
    alloc::{allocator::Kmalloc, IntoIter, KVec},
    prelude::GFP_KERNEL,
};

/// Represent a captured stacktrace of the current process
pub struct Stacktrace(KVec<u64>);

impl Stacktrace {
    /// Save a new stacktrace of the current process
    pub fn new(size: usize) -> Self {
        let mut buf = KVec::from_elem(0u64, size, GFP_KERNEL)?;

        let len = unsafe { bindings::stack_trace_save(buf.as_mut_ptr(), size as _, 0) };

        // SAFETY : We have by the `stack_trace_save` contract that `len<size` so `new_len<old_len`.
        unsafe { buf.set_len(len) };
        Stacktrace(buf)
    }
}

impl IntoIterator for Stacktrace {
    type IntoIter = IntoIter<u64, Kmalloc>;
    type Item = u64;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
