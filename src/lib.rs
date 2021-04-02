#![warn(missing_docs)]
//! Entangled provides thread pool based on the `async-executor` crate to spawn async futures on.
//!
//! It's main selling point is the "scoped spawn" functionality, which is essentially spawning
//! futures from the calling thread, which have access to the stack of the calling thread and joins
//! them after they have completed.
//!
//! # Example
//!
//! ```
//! use entangled::*;
//! use std::sync::atomic::*;
//!
//! let pool = ThreadPool::new(
//!     ThreadPoolDescriptor::default()
//! ).expect("can't create task pool");
//!
//! let counter = AtomicI32::new(0);
//! let ref_counter = &counter;
//!
//! pool.scope(|scope| {
//!     for _ in 0..10 {
//!         scope.spawn(async {
//!             ref_counter.fetch_add(1, Ordering::Relaxed);
//!         });
//!     }
//! });
//!
//! assert_eq!(counter.load(Ordering::Relaxed), 10);
//! ```

use std::sync::Arc;

#[doc(no_inline)]
pub use async_executor::Task;

/// Describes how a `ThreadPool` should be created.
#[derive(Debug, Clone)]
pub struct ThreadPoolDescriptor {
    /// Spawns at most n threads for the thread pool. Default: 2.
    pub num_threads: usize,
    /// The stack size of the spawned threads. Default: 2 MiB.
    pub stack_size: usize,
    /// Name of the threads. Threads will be named:
    /// {thread_name} ({thread index}), i.e. "Thread pool (0)"
    /// Default: "Thread pool"
    pub thread_name: String,
}

impl Default for ThreadPoolDescriptor {
    fn default() -> Self {
        Self {
            num_threads: 2,
            stack_size: 2 * 1024 * 1024,
            thread_name: "Thread pool".to_owned(),
        }
    }
}

/// Since the `ThreadPool` is Send + Sync, we need to wrap the threads into an inner struct,
/// so that we can track it's lifetime and properly can shutdown the threads on drop.
#[derive(Debug)]
struct ThreadPoolInner {
    threads: Vec<std::thread::JoinHandle<()>>,
    shutdown_tx: async_channel::Sender<()>,
}

impl Drop for ThreadPoolInner {
    fn drop(&mut self) {
        // Close the sender so that the shutdown is triggered.
        self.shutdown_tx.close();

        for join_handle in self.threads.drain(..) {
            let res = join_handle.join();
            if !std::thread::panicking() {
                res.expect("the task thread panicked while executing");
            }
        }
    }
}

/// A thread pool for executing futures.
///
/// Drives futures given to the tasks pool to completion.
#[derive(Debug, Clone)]
pub struct ThreadPool {
    executor: Arc<async_executor::Executor<'static>>,
    inner: Arc<ThreadPoolInner>,
}

impl ThreadPool {
    /// Create a new `ThreadPool`.
    pub fn new(descriptor: ThreadPoolDescriptor) -> Result<Self, std::io::Error> {
        let (shutdown_tx, shutdown_rx) = async_channel::unbounded::<()>();

        let executor = Arc::new(async_executor::Executor::new());
        let mut threads = Vec::with_capacity(descriptor.num_threads);

        for i in 0..descriptor.num_threads {
            let ex = Arc::clone(&executor);
            let shutdown_rx = shutdown_rx.clone();

            let thread_name = format!("{} ({})", descriptor.thread_name, i);

            let mut thread_builder = std::thread::Builder::new().name(thread_name);
            thread_builder = thread_builder.stack_size(descriptor.stack_size);

            let thread = thread_builder.spawn(move || {
                let shutdown_future = ex.run(shutdown_rx.recv());
                // We expect an async_channel::TryRecvError::Closed
                futures_lite::future::block_on(shutdown_future).unwrap_err();
            })?;

            threads.push(thread)
        }

        Ok(Self {
            executor,
            inner: Arc::new(ThreadPoolInner {
                threads,
                shutdown_tx,
            }),
        })
    }

    /// Creates a "fork-join" scope `s` and invokes the closure with a reference to `s`.
    /// This closure can then spawn futures into `s`. When the closure returns, it will block
    /// until all futures that have been spawned into `s` complete.
    ///
    /// In general, spawned futures may access stack data in place that outlives the scope itself.
    /// Other data must be fully owned by the spawned future.
    pub fn scope<'scope, S, R>(&self, s: S) -> Vec<R>
    where
        S: FnOnce(&mut Scope<'scope, R>) + 'scope + Send,
        R: Send + 'static,
    {
        // We transmute the lifetime of the executor to the lifetime of the scope.
        let executor = &*self.executor;
        let executor: &'scope async_executor::Executor = unsafe { std::mem::transmute(executor) };

        let mut scope = Scope {
            executor,
            spawned_tasks: Vec::new(),
        };

        // We call the callback `s`, which will return the spawned tasks.
        s(&mut scope);

        if scope.spawned_tasks.is_empty() {
            // Nothing to do.
            Vec::with_capacity(0)
        } else if scope.spawned_tasks.len() == 1 {
            // Only one task to create, so we can drive it to completion directly.
            vec![futures_lite::future::block_on(&mut scope.spawned_tasks[0])]
        } else {
            let mut futures = async move {
                let mut future_results = Vec::with_capacity(scope.spawned_tasks.len());
                for task in scope.spawned_tasks {
                    future_results.push(task.await);
                }
                future_results
            };

            // Pin the futures so that they don't move, and can thus be relied upon.
            let futures = unsafe { core::pin::Pin::new_unchecked(&mut futures) };

            // We transmute the lifetime of the futures from 'scope to 'static so that
            // we can spawn then on the thread pool. This is only safe, because we
            // make sure to drive them to completion until we exit the function.
            let futures: std::pin::Pin<&mut dyn futures_lite::Future<Output = Vec<R>>> = futures;
            let mut futures: std::pin::Pin<
                &'static mut (dyn futures_lite::Future<Output = Vec<R>> + 'static),
            > = unsafe { std::mem::transmute(futures) };

            // We also use the calling thread to drive the futures to completion.
            loop {
                if let Some(result) =
                    futures_lite::future::block_on(futures_lite::future::poll_once(&mut futures))
                {
                    break result;
                };

                self.executor.try_tick();
            }
        }
    }

    /// Spawns a static future onto the thread pool. The returned `Task` is a future. It can also be
    /// cancelled and "detached" allowing it to continue running without having to be polled by the
    /// end-user.
    pub fn spawn<T>(
        &self,
        future: impl futures_lite::Future<Output = T> + Send + 'static,
    ) -> async_executor::Task<T>
    where
        T: Send + 'static,
    {
        self.executor.spawn(future)
    }
}

/// Scopes the execution of futures.
#[derive(Debug)]
pub struct Scope<'scope, R> {
    executor: &'scope async_executor::Executor<'scope>,
    spawned_tasks: Vec<async_executor::Task<R>>,
}

impl<'scope, T: Send + 'scope> Scope<'scope, T> {
    /// `spawn` is similar to the spawn function in Rust's standard library. The difference is that
    /// we spawn a future and it is scoped, meaning that it's guaranteed to terminate before the
    /// current stack frame goes away, allowing you to reference the parent stack frame directly.
    ///
    /// This is ensured by having the parent thread join on the child futures before the scope exits.
    pub fn spawn<Fut: futures_lite::Future<Output = T> + 'scope + Send>(&mut self, f: Fut) {
        let task = self.executor.spawn(f);
        self.spawned_tasks.push(task);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI32, Ordering};

    use super::*;

    #[test]
    pub fn test_scoped_spawn() {
        let pool = ThreadPool::new(ThreadPoolDescriptor::default()).unwrap();

        let boxed = Box::new(100);
        let boxed_ref = &*boxed;

        let counter = Arc::new(AtomicI32::new(0));

        let outputs = pool.scope(|scope| {
            for _ in 0..100 {
                let count_clone = counter.clone();
                scope.spawn(async move {
                    if *boxed_ref != 100 {
                        panic!("expected 100")
                    } else {
                        count_clone.fetch_add(1, Ordering::Relaxed);
                        *boxed_ref
                    }
                });
            }
        });

        for output in &outputs {
            assert_eq!(*output, 100);
        }

        assert_eq!(outputs.len(), 100);
        assert_eq!(counter.load(Ordering::Relaxed), 100);
    }

    #[test]
    pub fn test_task_spawn() {
        let pool = ThreadPool::new(ThreadPoolDescriptor::default()).unwrap();

        let task = pool.spawn(async { 42 });

        assert_eq!(futures_lite::future::block_on(task), 42);
    }
}
