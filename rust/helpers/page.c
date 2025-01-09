// SPDX-License-Identifier: GPL-2.0

#include "asm/pgtable.h"
#include "asm/pgtable_types.h"
#include "linux/mm.h"
#include "linux/types.h"
#include <linux/gfp.h>
#include <linux/highmem.h>
#include <linux/smp.h>
#include <linux/pgtable.h>
#include <linux/page-flags.h>

struct page *rust_helper_alloc_pages(gfp_t gfp_mask, unsigned int order)
{
	return alloc_pages(gfp_mask, order);
}

void *rust_helper_kmap_local_page(struct page *page)
{
	return kmap_local_page(page);
}

void rust_helper_kunmap_local(const void *addr)
{
	kunmap_local(addr);
}
// Code taken from OSdev wiki to flush the TLB (onn one CPU but I consider that
// enought)
void rust_helper___native_flush_tlb_single(void *addr)
{
	asm volatile("invlpg (%0)" ::"r"(addr) : "memory");
}

void rust_helper_flush_tlb_each_cpu(unsigned long addr)
{
	on_each_cpu(rust_helper___native_flush_tlb_single, (void *)addr, 1);
}

unsigned long rust_helper_pmd_pfn(pmd_t pmd)
{
	return pmd_pfn(pmd);
}

unsigned long rust_helper_pte_pfn(pte_t pte)
{
	return pte_pfn(pte);
}

pgprot_t rust_helper_pmd_pgprot(pmd_t pmd)
{
	return pmd_pgprot(pmd);
}

pgprot_t rust_helper_pte_pgprot(pte_t pte)
{
	return pte_pgprot(pte);
}

void rust_helper_set_pmd(pmd_t *pmdp, pmd_t pmd)
{
	set_pmd(pmdp, pmd);
}

void rust_helper_set_pte(pte_t *ptep, pte_t pte)
{
	set_pte(ptep, pte);
}

pmd_t rust_helper_pfn_pmd(unsigned long pfn, pgprot_t pgprot)
{
	return pfn_pmd(pfn, pgprot);
}

pte_t rust_helper_pfn_pte(unsigned long pfn, pgprot_t pgprot)
{
	return pfn_pte(pfn, pgprot);
}

int rust_helper_page_high_mem(const struct page *page)
{
	return PageHighMem(page);
}

void *rust_helper_lowmem_page_address(const struct page *page)
{
	return lowmem_page_address(page);
}
