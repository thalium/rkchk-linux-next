// SPDX-License-Identifier: GPL-2.0

//! Kernel page allocation and management.

use crate::prelude::GFP_KERNEL;
use crate::{
    alloc::{AllocError, Flags, KVec},
    bindings,
    error::{code::*, Result},
    uaccess::UserSliceReader,
};
use core::{
    ptr::{self, NonNull},
    slice,
};

/// A bitwise shift for the page size.
pub const PAGE_SHIFT: usize = bindings::PAGE_SHIFT as usize;

/// The number of bytes in a page.
pub const PAGE_SIZE: usize = bindings::PAGE_SIZE;

/// A bitmask that gives the page containing a given address.
pub const PAGE_MASK: usize = !(PAGE_SIZE - 1);

/// Round up the given number to the next multiple of [`PAGE_SIZE`].
///
/// It is incorrect to pass an address where the next multiple of [`PAGE_SIZE`] doesn't fit in a
/// [`usize`].
pub const fn page_align(addr: usize) -> usize {
    // Parentheses around `PAGE_SIZE - 1` to avoid triggering overflow sanitizers in the wrong
    // cases.
    (addr + (PAGE_SIZE - 1)) & PAGE_MASK
}

/// Round down the given number to the next multiple of `size`.
///
/// It is incorrect to pass an address where the previous multiple of `size` doesn't fit in a
/// [`usize`].
pub fn page_align_down(addr: usize, size: usize) -> usize {
    addr - (addr % size)
}

/// A pointer to a page that owns the page allocation.
///
/// # Invariants
///
/// The pointer is valid, and has ownership over the page.
pub struct Page {
    page: NonNull<bindings::page>,
    order: u32,
}

// SAFETY: Pages have no logic that relies on them staying on a given thread, so moving them across
// threads is safe.
unsafe impl Send for Page {}

// SAFETY: Pages have no logic that relies on them not being accessed concurrently, so accessing
// them concurrently is safe.
unsafe impl Sync for Page {}

impl Page {
    /// Allocates a new page.
    ///
    /// # Examples
    ///
    /// Allocate memory for a page.
    ///
    /// ```
    /// use kernel::page::Page;
    ///
    /// # fn dox() -> Result<(), kernel::alloc::AllocError> {
    /// let page = Page::alloc_page(GFP_KERNEL)?;
    /// # Ok(()) }
    /// ```
    ///
    /// Allocate memory for a page and zero its contents.
    ///
    /// ```
    /// use kernel::page::Page;
    ///
    /// # fn dox() -> Result<(), kernel::alloc::AllocError> {
    /// let page = Page::alloc_page(GFP_KERNEL | __GFP_ZERO)?;
    /// # Ok(()) }
    /// ```
    pub fn alloc_page(flags: Flags) -> Result<Self, AllocError> {
        // SAFETY: Depending on the value of `gfp_flags`, this call may sleep. Other than that, it
        // is always safe to call this method.
        let page = unsafe { bindings::alloc_pages(flags.as_raw(), 0) };
        let page = NonNull::new(page).ok_or(AllocError)?;
        // INVARIANT: We just successfully allocated a page, so we now have ownership of the newly
        // allocated page. We transfer that ownership to the new `Page` object.
        Ok(Self { page, order: 0 })
    }

    /// Allocate 1 << order contiguous new pages.
    /// The physical address of the first page is naturally aligned
    /// (eg an order-3 allocation will be aligned to a multiple of 8 * PAGE_SIZE bytes).
    /// The NUMA policy of the current process is honoured when in process context.
    pub fn alloc_pages(flags: Flags, order: u32) -> Result<Self, AllocError> {
        // SAFETY: Depending on the value of `gfp_flags`, this call may sleep. Other than that, it
        // is always safe to call this method.
        let page = unsafe { bindings::alloc_pages(flags.as_raw(), order as _) };
        let page = NonNull::new(page).ok_or(AllocError)?;
        // INVARIANT: We just successfully allocated a page, so we now have ownership of the newly
        // allocated page. We transfer that ownership to the new `Page` object.
        Ok(Self { page, order })
    }

    /// Returns a raw pointer to the page.
    pub fn as_ptr(&self) -> *mut bindings::page {
        self.page.as_ptr()
    }

    /// Runs a piece of code with this page mapped to an address.
    ///
    /// The page is unmapped when this call returns.
    ///
    /// # Using the raw pointer
    ///
    /// It is up to the caller to use the provided raw pointer correctly. The pointer is valid for
    /// `PAGE_SIZE` bytes and for the duration in which the closure is called. The pointer might
    /// only be mapped on the current thread, and when that is the case, dereferencing it on other
    /// threads is UB. Other than that, the usual rules for dereferencing a raw pointer apply: don't
    /// cause data races, the memory may be uninitialized, and so on.
    ///
    /// If multiple threads map the same page at the same time, then they may reference with
    /// different addresses. However, even if the addresses are different, the underlying memory is
    /// still the same for these purposes (e.g., it's still a data race if they both write to the
    /// same underlying byte at the same time).
    fn with_page_mapped<T>(&self, f: impl FnOnce(*mut u8) -> T) -> T {
        // SAFETY: `page` is valid due to the type invariants on `Page`.
        let mapped_addr = unsafe { bindings::kmap_local_page(self.as_ptr()) };

        let res = f(mapped_addr.cast());

        // This unmaps the page mapped above.
        //
        // SAFETY: Since this API takes the user code as a closure, it can only be used in a manner
        // where the pages are unmapped in reverse order. This is as required by `kunmap_local`.
        //
        // In other words, if this call to `kunmap_local` happens when a different page should be
        // unmapped first, then there must necessarily be a call to `kmap_local_page` other than the
        // call just above in `with_page_mapped` that made that possible. In this case, it is the
        // unsafe block that wraps that other call that is incorrect.
        unsafe { bindings::kunmap_local(mapped_addr) };

        res
    }

    /// Runs a piece of code with a raw pointer to a slice of this page, with bounds checking.
    ///
    /// If `f` is called, then it will be called with a pointer that points at `off` bytes into the
    /// page, and the pointer will be valid for at least `len` bytes. The pointer is only valid on
    /// this task, as this method uses a local mapping.
    ///
    /// If `off` and `len` refers to a region outside of this page, then this method returns
    /// [`EINVAL`] and does not call `f`.
    ///
    /// # Using the raw pointer
    ///
    /// It is up to the caller to use the provided raw pointer correctly. The pointer is valid for
    /// `len` bytes and for the duration in which the closure is called. The pointer might only be
    /// mapped on the current thread, and when that is the case, dereferencing it on other threads
    /// is UB. Other than that, the usual rules for dereferencing a raw pointer apply: don't cause
    /// data races, the memory may be uninitialized, and so on.
    ///
    /// If multiple threads map the same page at the same time, then they may reference with
    /// different addresses. However, even if the addresses are different, the underlying memory is
    /// still the same for these purposes (e.g., it's still a data race if they both write to the
    /// same underlying byte at the same time).
    fn with_pointer_into_page<T>(
        &self,
        off: usize,
        len: usize,
        f: impl FnOnce(*mut u8) -> Result<T>,
    ) -> Result<T> {
        let bounds_ok = off <= PAGE_SIZE && len <= PAGE_SIZE && (off + len) <= PAGE_SIZE;

        if bounds_ok {
            self.with_page_mapped(move |page_addr| {
                // SAFETY: The `off` integer is at most `PAGE_SIZE`, so this pointer offset will
                // result in a pointer that is in bounds or one off the end of the page.
                f(unsafe { page_addr.add(off) })
            })
        } else {
            Err(EINVAL)
        }
    }

    /// Maps the page and reads from it into the given buffer.
    ///
    /// This method will perform bounds checks on the page offset. If `offset .. offset+len` goes
    /// outside of the page, then this call returns [`EINVAL`].
    ///
    /// # Safety
    ///
    /// * Callers must ensure that `dst` is valid for writing `len` bytes.
    /// * Callers must ensure that this call does not race with a write to the same page that
    ///   overlaps with this read.
    pub unsafe fn read_raw(&self, dst: *mut u8, offset: usize, len: usize) -> Result {
        self.with_pointer_into_page(offset, len, move |src| {
            // SAFETY: If `with_pointer_into_page` calls into this closure, then
            // it has performed a bounds check and guarantees that `src` is
            // valid for `len` bytes.
            //
            // There caller guarantees that there is no data race.
            unsafe { ptr::copy_nonoverlapping(src, dst, len) };
            Ok(())
        })
    }

    /// Maps the page and writes into it from the given buffer.
    ///
    /// This method will perform bounds checks on the page offset. If `offset .. offset+len` goes
    /// outside of the page, then this call returns [`EINVAL`].
    /// So this method only work with a write which is contained in the first page of the allocation.
    ///
    /// # Safety
    ///
    /// * Callers must ensure that `src` is valid for reading `len` bytes.
    /// * Callers must ensure that this call does not race with a read or write to the same page
    ///   that overlaps with this write.
    pub unsafe fn write_raw(&self, src: *const u8, offset: usize, len: usize) -> Result {
        self.with_pointer_into_page(offset, len, move |dst| {
            // SAFETY: If `with_pointer_into_page` calls into this closure, then it has performed a
            // bounds check and guarantees that `dst` is valid for `len` bytes.
            //
            // There caller guarantees that there is no data race.
            unsafe { ptr::copy_nonoverlapping(src, dst, len) };
            Ok(())
        })
    }

    fn for_each_pointer_into_page_mapped<T>(
        &self,
        off: usize,
        len: usize,
        init: T,
        mut f: impl FnMut(T, *mut u8, usize, usize) -> Result<T>,
    ) -> Result<T> {
        let n_pages = if len % PAGE_SIZE == 0 {
            len / PAGE_SIZE
        } else {
            len / PAGE_SIZE + 1
        };
        let off_pages = off / PAGE_SIZE;
        let off_in_page = off % PAGE_SIZE;

        let n_page_alloc = (2usize).pow(self.order);
        let bounds_ok = n_pages <= n_page_alloc && (off_pages + n_pages) <= n_page_alloc;

        if !bounds_ok {
            return Err(EINVAL);
        }

        // We have `first_len` <= PAGE_SIZE
        let first_len = PAGE_SIZE - off_in_page;
        // We have `last_len` <= PAGE_SIZE
        let last_len = (len - first_len) % PAGE_SIZE;

        let mut written: usize = 0;

        let mut ret = init;
        // We do the rest of the pages
        for i in off_pages..off_pages + n_pages {
            // SAFETY: `page` is valid due to the type invariants on `Page`.
            // `page` is an array of `n_page_alloc` pages due to type invariant.
            // We have 0 <= i < n_page_alloc due to the check realized above so the pointer is valid.
            let mapped_addr = unsafe { bindings::kmap_local_page(self.as_ptr().add(i)) };

            if i == off_pages {
                // We have that `first_len + off_in_page == PAGE_SIZE`
                // so `mapped_addr.add(off_in_page)` is valid for `first_len` bytes.
                // SAFETY : We have `off_in_page` <= `PAGE_SIZE`
                ret = f(
                    ret,
                    unsafe { (mapped_addr as *mut u8).add(off_in_page) },
                    first_len,
                    written,
                )?;

                written += first_len;
            }
            // We have n_pages >= 1 so can't underflow
            else if i == off_pages + n_pages - 1 {
                // We have `last_len` <= `PAGE_SIZE` and mapped_addr valid for `PAGE_SIZE` bytes
                // so valid for `last_len` bytes
                ret = f(ret, mapped_addr.cast(), last_len, written)?;

                written += last_len;
            } else {
                // The `mapped_addr` is valid for `PAGE_SIZE` bytes
                ret = f(ret, mapped_addr.cast(), PAGE_SIZE, written)?;

                written += PAGE_SIZE;
            }

            // This unmaps the page mapped above.
            //
            // SAFETY: Since this API takes the user code as a closure, it can only be used in a manner
            // where the pages are unmapped in reverse order. This is as required by `kunmap_local`.
            //
            // In other words, if this call to `kunmap_local` happens when a different page should be
            // unmapped first, then there must necessarily be a call to `kmap_local_page` other than the
            // call just above in `with_page_mapped` that made that possible. In this case, it is the
            // unsafe block that wraps that other call that is incorrect.
            unsafe { bindings::kunmap_local(mapped_addr) };
        }
        Ok(ret)
    }
    /// Maps each necessary pages from the allocatd pages and writes into it from the given buffer.
    ///
    /// This method will perform bounds checks on the offset and len asked. If `offset .. offset+len` goes
    /// outside of the allocation range, then this call returns [`EINVAL`].
    /// So this function also work for write spanning accross multiples pages.
    ///
    /// # Safety
    ///
    /// * Callers must ensure that `src` is valid for reading `len` bytes.
    /// * Callers must ensure that this call does not race with a read or write to the same page
    ///   that overlaps with this write.
    pub unsafe fn write_raw_multiple(&self, src: *const u8, offset: usize, len: usize) -> Result {
        self.for_each_pointer_into_page_mapped(offset, len, (), move |_, dst, page_len, written| {
            // SAFETY: If `for_each_pointer_into_page_mapped` calls into this closure, then it has performed a
            // bounds check and guarantees that `dst` is valid for `page_len` bytes.
            //
            // There caller guarantees that there is no data race.
            // There caller guarentees that src + written is valid for reading `page_len` bytes because
            // `for_each_pointer_into_page_mapped` guarantee that `written + page_len < len`
            unsafe { ptr::copy_nonoverlapping(src.add(written), dst, page_len) };
            Ok(())
        })
    }

    /// Compare the multiple allocated page with a same size allocation    
    pub unsafe fn compare_raw_multiple(
        &self,
        src: *const u8,
        offset: usize,
        len: usize,
    ) -> Result<KVec<*const u8>> {
        self.for_each_pointer_into_page_mapped::<KVec<*const u8>>(
            offset,
            len,
            KVec::new(),
            move |mut acc, dst, page_len, written| {
                let src_slice = unsafe { slice::from_raw_parts(src.add(written), page_len) };
                let dst_slice = unsafe { slice::from_raw_parts(dst, page_len) };

                for (i, e_src) in src_slice.iter().enumerate() {
                    let e_dst = unsafe { dst_slice.get_unchecked(i) };
                    if e_dst != e_src {
                        unsafe { acc.push(src.add(written).add(i), GFP_KERNEL)? };
                    }
                }
                Ok(acc)
            },
        )
    }

    /// Maps the page and zeroes the given slice.
    ///
    /// This method will perform bounds checks on the page offset. If `offset .. offset+len` goes
    /// outside of the page, then this call returns [`EINVAL`].
    ///
    /// # Safety
    ///
    /// Callers must ensure that this call does not race with a read or write to the same page that
    /// overlaps with this write.
    pub unsafe fn fill_zero_raw(&self, offset: usize, len: usize) -> Result {
        self.with_pointer_into_page(offset, len, move |dst| {
            // SAFETY: If `with_pointer_into_page` calls into this closure, then it has performed a
            // bounds check and guarantees that `dst` is valid for `len` bytes.
            //
            // There caller guarantees that there is no data race.
            unsafe { ptr::write_bytes(dst, 0u8, len) };
            Ok(())
        })
    }

    /// Copies data from userspace into this page.
    ///
    /// This method will perform bounds checks on the page offset. If `offset .. offset+len` goes
    /// outside of the page, then this call returns [`EINVAL`].
    ///
    /// Like the other `UserSliceReader` methods, data races are allowed on the userspace address.
    /// However, they are not allowed on the page you are copying into.
    ///
    /// # Safety
    ///
    /// Callers must ensure that this call does not race with a read or write to the same page that
    /// overlaps with this write.
    pub unsafe fn copy_from_user_slice_raw(
        &self,
        reader: &mut UserSliceReader,
        offset: usize,
        len: usize,
    ) -> Result {
        self.with_pointer_into_page(offset, len, move |dst| {
            // SAFETY: If `with_pointer_into_page` calls into this closure, then it has performed a
            // bounds check and guarantees that `dst` is valid for `len` bytes. Furthermore, we have
            // exclusive access to the slice since the caller guarantees that there are no races.
            reader.read_raw(unsafe { core::slice::from_raw_parts_mut(dst.cast(), len) })
        })
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        // SAFETY: By the type invariants, we have ownership of the page and can free it.
        unsafe { bindings::__free_pages(self.page.as_ptr(), self.order) };
    }
}
