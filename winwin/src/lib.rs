use allocator_api2::alloc::AllocError;
use allocator_api2::alloc::Allocator;
use std::cell::Cell;
use std::collections::HashMap;
use std::{alloc, ptr::NonNull};

pub use winwin_common::{Key, KeyState};

mod events;
pub use events::*;

mod wm;
pub use wm::*;

pub struct Context {
    // Arena allocator for temporary (frame) allocations.
    arena: Arena,
    // Presistent data uses global allocator.
    monitor_configs: HashMap<Monitor, Layout>,
}

impl Context {
    pub fn new() -> Self {
        Self {
            arena: Arena::new_unbounded(),
            monitor_configs: HashMap::new(),
        }
    }
}

pub struct Arena {
    mem: NonNull<u8>,
    end: Cell<usize>,
    used: Cell<usize>,
    capacity: usize,
}

impl Arena {
    pub fn new_unbounded() -> Self {
        // Overallocate. Commited memory should never be this high unless there was a leak.
        let size = u32::MAX as usize;
        let layout = alloc::Layout::array::<u8>(size).expect("arguments are correct");
        let mem = unsafe { alloc::alloc(layout) };
        let mem = NonNull::new(mem).expect("global alloc should not fail");

        Arena {
            mem,
            end: Cell::new(0),
            used: Cell::new(0),
            capacity: size,
        }
    }

    pub fn new_in(base: &mut [u8]) -> Self {
        todo!()
    }

    pub fn reset(&self) {
        self.end.set(0);
        self.used.set(0);
    }

    pub fn buff<T>(&self, size: usize) -> &mut [T] {
        let layout = alloc::Layout::array::<T>(size).unwrap();
        let mem = self.allocate(layout).unwrap();
        unsafe { std::slice::from_raw_parts_mut(mem.as_ptr() as *mut _, size) }
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        let layout = alloc::Layout::array::<u8>(self.capacity).expect("arguments are correct");
        unsafe { alloc::dealloc(self.mem.as_ptr(), layout) };
    }
}

unsafe impl Allocator for &Arena {
    fn allocate(&self, layout: alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        unsafe {
            let end = self.end.get();
            let curr_ptr = self.mem.as_ptr().add(end);
            let size = layout.size();
            let align = layout.align();

            let offset = curr_ptr.align_offset(align);
            if offset == usize::MAX || end + offset + size > self.capacity {
                return Err(AllocError);
            }

            let aligned_ptr = curr_ptr.add(offset);
            self.end.set(end + offset + size);
            self.used.set(self.used.get() + size);

            Ok(NonNull::slice_from_raw_parts(
                NonNull::new_unchecked(aligned_ptr),
                size,
            ))
        }
    }

    unsafe fn deallocate(&self, _ptr: NonNull<u8>, layout: alloc::Layout) {
        // Once all allocations are freed we reset this arena.
        let size = layout.size();
        self.used.set(self.used.get() - size);

        if self.used.get() == 0 {
            self.end.set(0);
        }
    }
}
