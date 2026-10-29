//! mimalloc, with plain `mi_malloc` for the usual alignments.
//!
//! `mimalloc::MiMalloc` calls `mi_malloc_aligned` for everything. In mimalloc v3, when a size
//! class has no free block at hand, an aligned allocation of a size that isn't a power of two
//! (a 48 byte object entry, say) is over-allocated from a bigger class instead. That skips the
//! path that collects the class's freed blocks, so it keeps happening. Every mimalloc block is
//! at least 8 byte aligned, so those don't need the aligned calls.

use std::alloc::{GlobalAlloc, Layout};
use std::ffi::c_void;

use libmimalloc_sys::{mi_free, mi_malloc, mi_realloc, mi_zalloc};
use mimalloc::MiMalloc;

/// What `mi_malloc` gives without asking.
const ALIGNED: usize = 8;

pub struct RouterGlobalAllocator;

unsafe impl GlobalAlloc for RouterGlobalAllocator {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.align() <= ALIGNED {
            mi_malloc(layout.size()) as *mut u8
        } else {
            MiMalloc.alloc(layout)
        }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if layout.align() <= ALIGNED {
            mi_zalloc(layout.size()) as *mut u8
        } else {
            MiMalloc.alloc_zeroed(layout)
        }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        mi_free(ptr as *mut c_void);
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if layout.align() <= ALIGNED {
            mi_realloc(ptr as *mut c_void, new_size) as *mut u8
        } else {
            MiMalloc.realloc(ptr, layout, new_size)
        }
    }
}
