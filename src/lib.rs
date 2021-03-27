//! Entangled is a simple threadpool with minimal dependencies. The main use case is a scoped fork-join,
//! i.e. spawning tasks from a single thread and having that thread await the completion of those tasks.
//! There are also utilities for generating the tasks from a slice of data.
//!
//! This library makes no attempt to ensure fairness or ordering of spawned tasks.
//!
//! This is a hard fork of the `bevy_tasks` crate, to further reduce dependencies and simplify the
//! interface.
//!
//! # Example
//!
//! ```
//! let pool = entangled::TaskPool::default();
//!
//! let count = std::sync::atomic::AtomicI32::new(0);
//! let ref_count = &count;
//!
//! let output = pool.scope(|scope| {
//!     for _ in 0..100 {
//!         scope.spawn(async {
//!             ref_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
//!             1
//!         });
//!     }
//! });
//!
//! assert_eq!(output.iter().sum::<i32>(), count.load(std::sync::atomic::Ordering::Relaxed));
//! ```
mod slice;
pub use slice::{ParallelSlice, ParallelSliceMut};

mod task;
pub use task::Task;

mod task_pool;
pub use task_pool::{Scope, TaskPool, TaskPoolDescriptor};

mod iter;
pub use iter::ParallelIterator;
