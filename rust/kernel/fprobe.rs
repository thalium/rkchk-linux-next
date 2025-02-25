//! Fprobes (fonctions probing)
//!
//! C header: [`include/linux/fprobe.h`](../../../../include/linux/fprobe.h)

use crate::error::Result;
use crate::init::PinInit;
use crate::str::CStr;
use crate::try_pin_init;
use crate::types::{ForeignOwnable, Opaque};
use bindings::{fprobe, pt_regs};
use core::ffi::c_void;
use core::marker::PhantomData;
use core::pin::Pin;
use kernel::error::Error;
use macros::{pin_data, pinned_drop};

/// Wraps the kernel's `struct fprobe`

/// fprobe flags
pub mod flags {
    /// This fprobe is soft-disabled.
    pub const FTRACE_FL_DISABLED: u32 = bindings::FPROBE_FL_DISABLED;

    /// This fprobe handler will be shared with kprobes.
    /// This flag must be set before registering.
    pub const FPROBE_FL_KPROBE_SHARED: u32 = bindings::FPROBE_FL_KPROBE_SHARED;
}

/// ftrace_ops flags
pub mod ops_flag {
    /// Prevent callback recursion at the expense of a little more overhead
    pub const FTRACE_OPS_FL_RECURSION: u32 = bindings::FTRACE_OPS_FL_RECURSION;
}

/// Correspond to the kernel `entry_handler` and `exit_handler` function for the fprobe function
///
/// You need to implement this trait each time you need a new callbacks
pub trait FprobeOperations
where
    Self: Sized,
{
    /// The global type that will be transmited to all the callbacks
    type Data: ForeignOwnable + Send + Sync;
    /// The type of the data that will be allocated at each entry of the hooked function
    /// and passed to the `entry_handler` and it's respective `exit_handler`
    type EntryData: Default + Sized;
    /// Callback called at each traced function entry
    fn entry_handler(
        data: <Self::Data as ForeignOwnable>::Borrowed<'_>,
        entry_ip: usize,
        ret_ip: usize,
        regs: &pt_regs,
        entry_data: Option<&mut Self::EntryData>,
    ) -> Option<()>;

    /// Callback called at each traced function exit
    fn exit_handler(
        data: <Self::Data as ForeignOwnable>::Borrowed<'_>,
        entry_ip: usize,
        ret_ip: usize,
        regs: &pt_regs,
        entry_data: Option<&mut Self::EntryData>,
    );
}

/// Represent the kernel's `struct fprobe` structure
///
/// # Invariants
///
///     `inner` is a registered fprobe
#[repr(transparent)]
#[pin_data(PinnedDrop)]
pub struct Fprobe<T: FprobeOperations> {
    #[pin]
    inner: Opaque<bindings::fprobe>,
    _t: PhantomData<T>,
}

// SAFETY: There is no `&self` methods
unsafe impl<T: FprobeOperations> Sync for Fprobe<T> where T::Data: Sync {}

// SAFETY: It is safe to unregister the probe on a different thread than
// the one used to register
unsafe impl<T: FprobeOperations> Send for Fprobe<T> where T::Data: Send {}

impl<T: FprobeOperations> Fprobe<T> {
    /// Create a new `struct fprobe` structure
    ///
    /// But it dont register it, to register it call `register()`
    fn new_inner(data: T::Data) -> bindings::fprobe {
        let mut ops = bindings::ftrace_ops::default();
        ops.flags |= ops_flag::FTRACE_OPS_FL_RECURSION as u64;
        ops.private = data.into_foreign() as *mut c_void;
        bindings::fprobe {
            ops,
            nmissed: 0,
            flags: 0,
            rethook: core::ptr::null_mut::<bindings::rethook>(),
            // We just need common data between all the function not only the entry and it's corresponding exit
            entry_data_size: core::mem::size_of::<T::EntryData>(),
            nr_maxactive: 50,
            entry_handler: Some(Fprobe::<T>::entry_handler_callback as _),
            exit_handler: Some(Fprobe::<T>::exit_handler_callback as _),
        }
    }

    /// Create a new `struct fprobe` structure and register it
    pub fn new(
        filter: &'static CStr,
        notfilter: Option<&'static CStr>,
        private_data: T::Data,
    ) -> impl PinInit<Self, Error> {
        try_pin_init!(Self {
            inner <- Opaque::try_ffi_init(move |slot: *mut bindings::fprobe| {
                // SAFETY: The initializer can write to the provided `slot`.
                unsafe { slot.write(Self::new_inner(private_data))};

                // SAFETY: We wrote the data to the fprobe structure.
                // We have the fprobe structure pinned to our type so will be unregistred
                // before being deallocated
                // INVARIANT: If this return `Ok(())`, then the `slot` will contan a registred
                // device
                unsafe {
                    Self::register(slot, filter, notfilter)
                }
            }),
            _t: PhantomData,
        })
    }

    /// # Safety
    ///     Will be called only from C, prototype correspond to the fprobe's callback prototype
    unsafe extern "C" fn entry_handler_callback(
        fp: *mut fprobe,
        entry_ip: core::ffi::c_ulong,
        ret_ip: core::ffi::c_ulong,
        regs: *mut pt_regs,
        entry_data: *mut core::ffi::c_void,
    ) -> core::ffi::c_int {
        let mut entry_ref = None;

        let entry_data = entry_data as *mut T::EntryData;
        if !entry_data.is_null() {
            // SAFETY: We write the size we asked for and was allocated according
            // to the fprobe API. This is a private field or this specific
            // call to the hooked function that will only be used at the exit of
            // the function so there is not concurrency problem
            unsafe { entry_data.write(T::EntryData::default()) };

            // SAFETY: The pointer is not null and aligned according to the kernel allocator
            // garanties
            unsafe { entry_ref = Some(&mut *entry_data) };
        }

        // SAFETY: This callback is called only when the fprobe structure is still registered
        // So the ops.private field is still valid
        let data = unsafe { T::Data::borrow((*fp).ops.private) };

        // SAFETY: The pointer is created at the call of our callback so no need to chack for race
        // However writting to it has side effect so we set it to non mutable
        let regs = unsafe { &*regs };

        match T::entry_handler(data, entry_ip as usize, ret_ip as usize, regs, entry_ref) {
            Some(()) => 0,
            None => -1,
        }
    }

    /// # Safety
    ///     Will be called only from C, prototype correspond to the fprobe's callback prototype
    unsafe extern "C" fn exit_handler_callback(
        fp: *mut fprobe,
        entry_ip: core::ffi::c_ulong,
        ret_ip: core::ffi::c_ulong,
        regs: *mut pt_regs,
        entry_data: *mut core::ffi::c_void,
    ) {
        let mut entry_ref = None;

        let entry_data = entry_data as *mut T::EntryData;
        if !entry_data.is_null() {
            // SAFETY: The pointer is not null and aligned
            // according to the kernel allocator garanties
            unsafe { entry_ref = Some(&mut *entry_data) };
        }

        // SAFETY: This callback is called only when the fprobe structure is still registered
        // So the ops.private field is still valid
        let data = unsafe { <T::Data as ForeignOwnable>::borrow((*fp).ops.private) };

        // SAFETY: The pointer is created at the call of our callback so no need to chack for race
        // However writting to it has side effect so we set it to non mutable
        let regs = unsafe { &*regs };

        T::exit_handler(data, entry_ip as usize, ret_ip as usize, regs, entry_ref);
    }

    /// Register a `struct probe` to ftrace by pattern
    ///
    /// # Safety
    ///     `fp`` must be a properly filled fprobe structure
    unsafe fn register(
        fp: *mut bindings::fprobe,
        filter: &CStr,
        notfilter: Option<&CStr>,
    ) -> Result {
        let mut pnot_filter = core::ptr::null::<core::ffi::c_char>();
        if let Some(notfilter) = notfilter {
            pnot_filter = notfilter.as_char_ptr();
        }

        // SAFETY: fp is a well filled fprobe structure
        crate::error::to_result(unsafe {
            bindings::register_fprobe(fp, filter.as_char_ptr(), pnot_filter)
        })
    }
}

#[pinned_drop]
impl<T: FprobeOperations> PinnedDrop for Fprobe<T> {
    fn drop(self: Pin<&mut Self>) {
        // SAFETY: WWe know the fprobe is registered by the type invariant
        // The doc don't really specify why this call would fail so...
        crate::error::to_result(unsafe { bindings::unregister_fprobe(self.inner.get()) }).unwrap();
        // SAFETY: We unregistered the fprobe so no one hold a Borrowed reference to the pointer
        // and we are in the Drop Impl so this is the first and last call to `from_foreign` corresponding exactly with
        // the call to `into_foreign`
        unsafe {
            <<T as FprobeOperations>::Data as ForeignOwnable>::from_foreign(
                (*self.inner.get()).ops.private,
            )
        };
    }
}
