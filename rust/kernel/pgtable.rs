// SPDX-License-Identifier: GPL-2.0

//! Kernel page table management.

use core::ptr::NonNull;

use bindings::pgprot_t;

use crate::prelude::EINVAL;
use kernel::error::Result;

/// Utility function common tp the different page table level
pub trait Pgtable {
    /// Get the order of the table entry (correspond to the alloc_pages order)
    fn order(&self) -> u32;
    /// Get the PFN of the table entry
    fn pfn(&self) -> u64;
    /// Get the page property of the table entry
    fn pgprot(&self) -> pgprot_t;
    /// Set the current pgtable to the `new_pgtable`
    /// # SAFETY :
    ///     This call doesn't update the TLB caches, a call to `flush_tlb` should be made after this function
    ///     The new_pfn and new_pgprot should have been obtained from `pfn` and `pgprot` call of the same level page
    unsafe fn set_pgtable(&mut self, new_pfn: u64, new_pgprot: pgprot_t);
}

/// Represent a pointer to a page middle directory
///
/// # Invariant :
///     pmd point to a valid page middle directory
pub struct Pmd(NonNull<bindings::pmd_t>);

impl Pgtable for Pmd {
    fn order(&self) -> u32 {
        bindings::PMD_ORDER
    }
    fn pfn(&self) -> u64 {
        // SAFETY : According to the type invariant self.0 point to a valid pmd
        let pmd = unsafe { *self.0.as_ptr() };
        // SAFETY : Just an FFI call
        (unsafe { bindings::pmd_pfn(pmd) }) as u64
    }

    fn pgprot(&self) -> pgprot_t {
        // SAFETY : According to the type invariant self.0 point to a valid pmd
        let pmd = unsafe { *self.0.as_ptr() };
        // SAFETY : Just an FFI call
        unsafe { bindings::pmd_pgprot(pmd) }
    }

    unsafe fn set_pgtable(&mut self, new_pfn: u64, new_pgprot: pgprot_t) {
        // SAFETY : Just an FFI call
        let pmd = unsafe { bindings::pfn_pmd(new_pfn, new_pgprot) };
        // SAFETY : According to the safety ontrat of the trait function
        // we can change the value of the pmd
        unsafe { bindings::set_pmd(self.0.as_ptr(), pmd) };
    }
}

/// Represent a pointer to a page table entry
///
/// # Invariant :
///     pte point to a valid page table entry
pub struct Pte(NonNull<bindings::pte_t>);

impl Pgtable for Pte {
    fn order(&self) -> u32 {
        1
    }
    fn pfn(&self) -> u64 {
        // SAFETY : According to the type invariant self.0 point to a valid pmd
        let pte = unsafe { *self.0.as_ptr() };
        // SAFETY : Just an FFI call
        (unsafe { bindings::pte_pfn(pte) }) as u64
    }
    fn pgprot(&self) -> pgprot_t {
        // SAFETY : According to the type invariant self.0 point to a valid pmd
        let pte = unsafe { *self.0.as_ptr() };
        // SAFETY : Just an FFI call
        unsafe { bindings::pte_pgprot(pte) }
    }
    unsafe fn set_pgtable(&mut self, new_pfn: u64, new_pgprot: pgprot_t) {
        // SAFETY : Just an FFI call
        let pte = unsafe { bindings::pfn_pte(new_pfn, new_pgprot) };
        // SAFETY : According to the safety ontrat of the trait function
        // we can change the value of the pmd
        unsafe { bindings::set_pte(self.0.as_ptr(), pte) };
    }
}

/// The different page table level
pub enum PageLevel {
    /// PTE level, 4K page
    Pte(Pte),
    /// PMD level, 2M page
    Pmd(Pmd),
}

impl Pgtable for PageLevel {
    fn order(&self) -> u32 {
        match self {
            PageLevel::Pmd(pmd) => pmd.order(),
            PageLevel::Pte(pte) => pte.order(),
        }
    }
    fn pfn(&self) -> u64 {
        match self {
            PageLevel::Pmd(pmd) => pmd.pfn(),
            PageLevel::Pte(pte) => pte.pfn(),
        }
    }
    fn pgprot(&self) -> pgprot_t {
        match self {
            PageLevel::Pmd(pmd) => pmd.pgprot(),
            PageLevel::Pte(pte) => pte.pgprot(),
        }
    }
    unsafe fn set_pgtable(&mut self, new_pfn: u64, new_pgprot: pgprot_t) {
        match self {
            // SAFETY : By the safeyt contract of this function
            PageLevel::Pmd(pmd) => unsafe { pmd.set_pgtable(new_pfn, new_pgprot) },
            // SAFETY : By the safeyt contract of this function
            PageLevel::Pte(pte) => unsafe { pte.set_pgtable(new_pfn, new_pgprot) },
        }
    }
}

/// Lookup for the page at the address `address`
pub fn lookup_address(address: usize) -> Result<PageLevel> {
    let mut level: u32 = 0;
    // SAFETY : Just an FFI call, `&mut level` is not null
    let ptr = unsafe { bindings::lookup_address(address as _, &mut level as *mut u32) };
    if ptr.is_null() {
        return Err(EINVAL);
    }
    match level {
        bindings::pg_level_PG_LEVEL_4K => Ok(PageLevel::Pte(Pte(
            // SAFETY : `ptr` is not null, checked above
            // As the level indicate ptr point to a valid pte entry
            // according to the lookup_address contract
            unsafe { NonNull::new_unchecked(ptr) },
        ))),
        bindings::pg_level_PG_LEVEL_2M => Ok(PageLevel::Pmd(Pmd(
            // SAFETY : `ptr` is not null, checked above
            // As the level indicate ptr point to a valid pmd entry
            // according to the lookup_address contract
            unsafe { NonNull::new_unchecked(ptr as *mut bindings::pmd_t) },
        ))),
        _ => Err(EINVAL),
    }
}
