use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use futures_lite::{future, pin};

use crate::Task;

/// Describes how a `TaskPool` should be created.
#[derive(Debug, Clone)]
pub struct TaskPoolDescriptor {
    /// Will set up the thread pool to use at most n threads. Default: 2.
    pub num_threads: usize,
    /// Will use the given stack size in KiB. Default: 2 MiB.
    pub stack_size: usize,
    /// Name of the threads. Threads will be names:
    /// <thread_name> (<thread_index>), i.e. "MyThreadPool (2)"
    /// Default: "Thread pool"
    pub thread_name: String,
}

impl Default for TaskPoolDescriptor {
    fn default() -> Self {
        Self {
            num_threads: 2,
            stack_size: 2 * 1024 * 1024,
            thread_name: "Thread pool".to_owned(),
        }
    }
}

#[derive(Debug)]
struct TaskPoolInner {
    threads: Vec<JoinHandle<()>>,
    shutdown_tx: async_channel::Sender<()>,
}

impl Drop for TaskPoolInner {
    fn drop(&mut self) {
        self.shutdown_tx.close();

        let panicking = thread::panicking();
        for join_handle in self.threads.drain(..) {
            let res = join_handle.join();
            if !panicking {
                res.expect("Task thread panicked while executing.");
            }
        }
    }
}

/// A thread pool for executing tasks.
///
/// Tasks are futures that are being automatically driven by the pool on threads owned by the pool.
#[derive(Debug, Clone)]
pub struct TaskPool {
    /// The executor for the pool.
    ///
    /// This has to be separate from `TaskPoolInner` because we have to create an `Arc<Executor>` to
    /// pass into the worker threads, and we must create the worker threads before we can create
    /// the `Vec<Task<T>>` contained within `TaskPoolInner`.
    executor: Arc<async_executor::Executor<'static>>,

    /// Inner state of the pool.
    inner: Arc<TaskPoolInner>,
}

impl TaskPool {
    thread_local! {
        static LOCAL_EXECUTOR: async_executor::LocalExecutor<'static> = async_executor::LocalExecutor::new();
    }

    /// Create a new `TaskPool`.
    pub fn new(descriptor: &TaskPoolDescriptor) -> Self {
        let num_threads = descriptor.num_threads;
        let stack_size = descriptor.stack_size;
        let thread_name = &descriptor.thread_name;

        let (shutdown_tx, shutdown_rx) = async_channel::unbounded::<()>();

        let executor = Arc::new(async_executor::Executor::new());

        let threads = (0..num_threads)
            .map(|i| {
                let ex = Arc::clone(&executor);
                let shutdown_rx = shutdown_rx.clone();

                let thread_name = format!("{} ({})", thread_name, i);

                let mut thread_builder = thread::Builder::new().name(thread_name);
                thread_builder = thread_builder.stack_size(stack_size);

                thread_builder
                    .spawn(move || {
                        let shutdown_future = ex.run(shutdown_rx.recv());
                        // Use unwrap_err because we expect a Closed error
                        future::block_on(shutdown_future).unwrap_err();
                    })
                    .expect("Failed to spawn thread.")
            })
            .collect();

        Self {
            executor,
            inner: Arc::new(TaskPoolInner {
                threads,
                shutdown_tx,
            }),
        }
    }

    /// Return the number of threads owned by the `TaskPool`
    pub fn thread_num(&self) -> usize {
        self.inner.threads.len()
    }

    /// Allows spawning non-`static futures on the thread pool. The function takes a callback,
    /// passing a scope object into it. The scope object provided to the callback can be used
    /// to spawn tasks. This function will await the completion of all tasks before returning.
    ///
    /// This is similar to `rayon::scope` and `crossbeam::scope`.
    pub fn scope<'scope, F, T>(&self, f: F) -> Vec<T>
    where
        F: FnOnce(&mut Scope<'scope, T>) + 'scope + Send,
        T: Send + 'static,
    {
        TaskPool::LOCAL_EXECUTOR.with(|local_executor| {
            // SAFETY: This function blocks until all futures complete, so this future must return
            // before this function returns. However, rust has no way of knowing this so we must
            // convert to 'static here to appease the compiler as it is unable to validate safety.
            let executor: &async_executor::Executor = &*self.executor;
            let executor: &'scope async_executor::Executor = unsafe { mem::transmute(executor) };
            let local_executor: &'scope async_executor::LocalExecutor =
                unsafe { mem::transmute(local_executor) };
            let mut scope = Scope {
                executor,
                local_executor,
                spawned: Vec::new(),
            };

            f(&mut scope);

            if scope.spawned.is_empty() {
                Vec::default()
            } else if scope.spawned.len() == 1 {
                vec![future::block_on(&mut scope.spawned[0])]
            } else {
                let fut = async move {
                    let mut results = Vec::with_capacity(scope.spawned.len());
                    for task in scope.spawned {
                        results.push(task.await);
                    }

                    results
                };

                // Pin the futures on the stack.
                pin!(fut);

                // SAFETY: This function blocks until all futures complete, so we do not read/write
                // the data from futures outside of the 'scope lifetime. However, Rust has no way
                // of knowing this so we must convert to 'static here to appease the compiler as it
                // is unable to validate safety.
                let fut: Pin<&mut (dyn Future<Output = Vec<T>>)> = fut;
                let fut: Pin<&'static mut (dyn Future<Output = Vec<T>> + 'static)> =
                    unsafe { mem::transmute(fut) };

                // The thread that calls scope() will participate in driving tasks in the pool
                // forward until the tasks that are spawned by this scope() call complete.
                // (If the caller of scope() happens to be a thread in this thread pool, and we
                // only have one thread in the pool, then simply calling
                // future::block_on(spawned) would deadlock.)
                let mut spawned = local_executor.spawn(fut);
                loop {
                    if let Some(result) = future::block_on(future::poll_once(&mut spawned)) {
                        break result;
                    };

                    self.executor.try_tick();
                    local_executor.try_tick();
                }
            }
        })
    }

    /// Spawns a static future onto the thread pool. The returned `Task` is a future. It can also be
    /// cancelled and "detached" allowing it to continue running without having to be polled by the
    /// end-user.
    pub fn spawn<T>(&self, future: impl Future<Output = T> + Send + 'static) -> Task<T>
    where
        T: Send + 'static,
    {
        self.executor.spawn(future)
    }
}

impl Default for TaskPool {
    fn default() -> Self {
        Self::new(&TaskPoolDescriptor::default())
    }
}

/// Scopes the execution of tasks.
#[derive(Debug)]
pub struct Scope<'scope, T> {
    executor: &'scope async_executor::Executor<'scope>,
    local_executor: &'scope async_executor::LocalExecutor<'scope>,
    spawned: Vec<async_executor::Task<T>>,
}

impl<'scope, T: Send + 'scope> Scope<'scope, T> {
    /// `spawn` is similar to the spawn function in Rust's standard library. The difference is that
    /// we spawn a future and it is scoped, meaning that it's guaranteed to terminate before the
    /// current stack frame goes away, allowing you to reference the parent stack frame directly.
    ///
    /// This is ensured by having the parent thread join on the child futures before the scope exits.
    pub fn spawn<Fut: Future<Output = T> + 'scope + Send>(&mut self, f: Fut) {
        let task = self.executor.spawn(f);
        self.spawned.push(task);
    }

    /// Spawns a future on the thread-local executor.
    pub fn spawn_local<Fut: Future<Output = T> + 'scope>(&mut self, f: Fut) {
        let task = self.local_executor.spawn(f);
        self.spawned.push(task);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use std::sync::Barrier;

    use super::*;

    #[test]
    pub fn test_spawn() {
        let pool = TaskPool::default();

        let foobar = Box::new(42);
        let foobar = &*foobar;

        let count = Arc::new(AtomicI32::new(0));

        let outputs = pool.scope(|scope| {
            for _ in 0..100 {
                let count_clone = count.clone();
                scope.spawn(async move {
                    if *foobar != 42 {
                        panic!("not 42!?!?")
                    } else {
                        count_clone.fetch_add(1, Ordering::Relaxed);
                        *foobar
                    }
                });
            }
        });

        for output in &outputs {
            assert_eq!(*output, 42);
        }

        assert_eq!(outputs.len(), 100);
        assert_eq!(count.load(Ordering::Relaxed), 100);
    }

    #[test]
    pub fn test_mixed_spawn_local_and_spawn() {
        let pool = TaskPool::default();

        let foobar = Box::new(42);
        let foobar = &*foobar;

        let local_count = Arc::new(AtomicI32::new(0));
        let non_local_count = Arc::new(AtomicI32::new(0));

        let outputs = pool.scope(|scope| {
            for i in 0..100 {
                if i % 2 == 0 {
                    let count_clone = non_local_count.clone();
                    scope.spawn(async move {
                        if *foobar != 42 {
                            panic!("not 42!?!?")
                        } else {
                            count_clone.fetch_add(1, Ordering::Relaxed);
                            *foobar
                        }
                    });
                } else {
                    let count_clone = local_count.clone();
                    scope.spawn_local(async move {
                        if *foobar != 42 {
                            panic!("not 42!?!?")
                        } else {
                            count_clone.fetch_add(1, Ordering::Relaxed);
                            *foobar
                        }
                    });
                }
            }
        });

        for output in &outputs {
            assert_eq!(*output, 42);
        }

        assert_eq!(outputs.len(), 100);
        assert_eq!(local_count.load(Ordering::Relaxed), 50);
        assert_eq!(non_local_count.load(Ordering::Relaxed), 50);
    }

    #[test]
    pub fn test_thread_locality() {
        let pool = Arc::new(TaskPool::default());
        let count = Arc::new(AtomicI32::new(0));
        let barrier = Arc::new(Barrier::new(101));
        let thread_check_failed = Arc::new(AtomicBool::new(false));

        for _ in 0..100 {
            let inner_barrier = barrier.clone();
            let count_clone = count.clone();
            let inner_pool = pool.clone();
            let inner_thread_check_failed = thread_check_failed.clone();
            std::thread::spawn(move || {
                inner_pool.scope(|scope| {
                    let inner_count_clone = count_clone.clone();
                    scope.spawn(async move {
                        inner_count_clone.fetch_add(1, Ordering::Release);
                    });
                    let spawner = std::thread::current().id();
                    let inner_count_clone = count_clone.clone();
                    scope.spawn_local(async move {
                        inner_count_clone.fetch_add(1, Ordering::Release);
                        if std::thread::current().id() != spawner {
                            // NOTE: This check is using an atomic rather than simply panicing the
                            // thread to avoid deadlocking the barrier on failure
                            inner_thread_check_failed.store(true, Ordering::Release);
                        }
                    });
                });
                inner_barrier.wait();
            });
        }
        barrier.wait();
        assert!(!thread_check_failed.load(Ordering::Acquire));
        assert_eq!(count.load(Ordering::Acquire), 200);
    }

    #[test]
    pub fn test_task_spawn() {
        let pool = TaskPool::default();

        let task = pool.spawn(async { 42 });

        assert_eq!(future::block_on(task), 42);
    }
}
