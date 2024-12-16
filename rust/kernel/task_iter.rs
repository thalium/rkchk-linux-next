//! Implement iterator for the tasks's linked list
use core::iter::Iterator;

use crate::{task::Task, types::ARef};

/// Implement the Iterator trait for `ARef<Task>`
pub struct TaskIter {
    task: Option<ARef<Task>>,
    task_origin: ARef<Task>,
}

impl IntoIterator for ARef<Task> {
    type Item = ARef<Task>;
    type IntoIter = TaskIter;

    fn into_iter(self) -> Self::IntoIter {
        TaskIter {
            task: None,
            task_origin: self,
        }
    }
}

impl Iterator for TaskIter {
    type Item = ARef<Task>;
    fn next(&mut self) -> Option<Self::Item> {
        let next_task: *mut Task = if let Some(task) = &self.task {
            // We made it around the linked list
            if task.as_ptr() == self.task_origin.as_ptr() {
                return None;
            }
            // SAFETY: For the FFI call: the underlying macro
            // is safe to use and use the good synchronization macro
            // For the casting
            unsafe { bindings::next_task(task.as_ptr() as *const _) }.cast()
        } else {
            // SAFETY: For the FFI call: the underlying macro
            // is safe to use and use the good synchronization macro
            // For the casting

            unsafe { bindings::next_task(self.task_origin.as_ptr() as *const _) }.cast()
        };

        if !next_task.is_null() {
            // SAFETY: The reference is not null and aligned according to C guaranty
            // However it may be aliased...
            let next_task = ARef::from(unsafe { &*next_task });

            self.task = Some(next_task.clone());

            Some(next_task)
        } else {
            None
        }

        /*
        // SAFETY: The pointer is valid we just increment it's refcount
        unsafe { (*next_task).inc_ref() };

        let next_task = match NonNull::<Task>::new(next_task) {
            // SAFETY: We incrmeented the recount above
            Some(non_null) => unsafe { ARef::from_raw(non_null) },
            None => return None,
        };
        self.task = Some(next_task.clone());
        Some(next_task)
        */
    }
}
