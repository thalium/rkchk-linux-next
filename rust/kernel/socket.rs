//! A really quick and dirty wrapping for some function I need relative to sockets

use kernel::prelude::*;

/// Check if a file decriptor is a socket or not
pub fn is_fd_sock(fd: i32) -> Result<bool> {
    let mut err: i32 = 0;

    // SAFETY: We have err mutable and it's pointer is not null
    let sock =
        unsafe { bindings::sockfd_lookup(fd, &mut err as *mut i32 as *mut core::ffi::c_int) };

    if sock.is_null() {
        match err {
            -88 => Ok(false),
            _ => Err(Error::from_errno(err)),
        }
    } else {
        Ok(true)
    }
}
