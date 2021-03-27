use super::TaskPool;

/// Helper functions to iterate over a slice in parallel.
pub trait ParallelSlice<T: Sync>: AsRef<[T]> {
    /// Iterates in parallel over a slice with a given chunk size.
    ///
    /// # Example
    ///
    /// ```
    /// use entangled::*;
    ///
    /// let v = vec![42; 1000];
    /// let task_pool = TaskPool::default();
    /// let outputs = v.par_chunk_map(&task_pool, 512, |numbers| -> i32 { numbers.iter().sum() });
    /// let mut sum = 0;
    ///
    /// for output in outputs {
    ///     sum += output;
    /// }
    ///
    /// assert_eq!(sum, 1000 * 42);
    /// ```
    fn par_chunk_map<F, R>(&self, task_pool: &TaskPool, chunk_size: usize, f: F) -> Vec<R>
    where
        F: Fn(&[T]) -> R + Send + Sync,
        R: Send + 'static,
    {
        let slice = self.as_ref();
        let f = &f;
        task_pool.scope(|scope| {
            for chunk in slice.chunks(chunk_size) {
                scope.spawn(async move { f(chunk) });
            }
        })
    }

    /// Iterates in parallel over a slice with a given maximum of tasks. `max_tasks` is None, it
    /// will use the thread number of the `TaskPool`.
    ///
    /// # Example
    ///
    /// ```
    /// use entangled::*;
    ///
    /// let v = vec![42; 1000];
    /// let task_pool = TaskPool::default();
    /// let outputs = v.par_splat_map(&task_pool, Some(2), |numbers| -> i32 { numbers.iter().sum() });
    /// let mut sum = 0;
    ///
    /// for output in outputs {
    ///     sum += output;
    /// }
    ///
    /// assert_eq!(sum, 1000 * 42);
    /// ```
    fn par_splat_map<F, R>(&self, task_pool: &TaskPool, max_tasks: Option<usize>, f: F) -> Vec<R>
    where
        F: Fn(&[T]) -> R + Send + Sync,
        R: Send + 'static,
    {
        let slice = self.as_ref();
        let chunk_size = std::cmp::max(
            1,
            std::cmp::max(
                slice.len() / task_pool.thread_num(),
                slice.len() / max_tasks.unwrap_or(usize::MAX),
            ),
        );

        slice.par_chunk_map(task_pool, chunk_size, f)
    }
}

impl<S, T: Sync> ParallelSlice<T> for S where S: AsRef<[T]> {}

/// Helper functions to iterate over a mutable slice in parallel.
pub trait ParallelSliceMut<T: Send>: AsMut<[T]> {
    /// Iterates in parallel over a mutable slice with a given chunk size.
    ///
    /// # Example
    ///
    /// ```
    /// use entangled::*;
    ///
    /// let mut v = vec![42; 1000];
    /// let task_pool = TaskPool::default();
    ///
    /// let outputs = v.par_chunk_map_mut(&task_pool, 512, |numbers| -> i32 {
    /// for number in numbers.iter_mut() {
    ///     *number *= 2;
    /// }
    /// numbers.iter().sum()
    /// });
    ///
    /// let mut sum = 0;
    /// for output in outputs {
    ///     sum += output;
    /// }
    ///
    /// assert_eq!(sum, 1000 * 42 * 2);
    /// assert_eq!(v[0], 84);
    /// ```
    fn par_chunk_map_mut<F, R>(&mut self, task_pool: &TaskPool, chunk_size: usize, f: F) -> Vec<R>
    where
        F: Fn(&mut [T]) -> R + Send + Sync,
        R: Send + 'static,
    {
        let slice = self.as_mut();
        let f = &f;
        task_pool.scope(|scope| {
            for chunk in slice.chunks_mut(chunk_size) {
                scope.spawn(async move { f(chunk) });
            }
        })
    }

    /// Iterates in parallel over a mutable slice with a given maximum of tasks. `max_tasks` is None, it
    /// will use the thread number of the `TaskPool`.
    ///
    /// # Example
    ///
    /// ```
    /// use entangled::*;
    ///
    /// let mut v = vec![42; 1000];
    /// let task_pool = TaskPool::default();
    ///
    /// let outputs = v.par_splat_map_mut(&task_pool, None, |numbers| -> i32 {
    /// for number in numbers.iter_mut() {
    ///     *number *= 2;
    /// }
    /// numbers.iter().sum()
    /// });
    ///
    /// let mut sum = 0;
    /// for output in outputs {
    ///     sum += output;
    /// }
    ///
    /// assert_eq!(sum, 1000 * 42 * 2);
    /// assert_eq!(v[0], 84);
    /// ```
    fn par_splat_map_mut<F, R>(
        &mut self,
        task_pool: &TaskPool,
        max_tasks: Option<usize>,
        f: F,
    ) -> Vec<R>
    where
        F: Fn(&mut [T]) -> R + Send + Sync,
        R: Send + 'static,
    {
        let mut slice = self.as_mut();
        let chunk_size = std::cmp::max(
            1,
            std::cmp::max(
                slice.len() / task_pool.thread_num(),
                slice.len() / max_tasks.unwrap_or(usize::MAX),
            ),
        );

        slice.par_chunk_map_mut(task_pool, chunk_size, f)
    }
}

impl<S, T: Send> ParallelSliceMut<T> for S where S: AsMut<[T]> {}
