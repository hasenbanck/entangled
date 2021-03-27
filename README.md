# entangled
[![Latest version](https://img.shields.io/crates/v/entangled.svg)](https://crates.io/crates/entangled)
[![Documentation](https://docs.rs/entangled/badge.svg)](https://docs.rs/entangled)
![MIT](https://img.shields.io/badge/license-MIT-blue.svg)

entangled is a simple thread pool with minimal dependencies. The main use case is a scoped
fork-join, i.e. spawning tasks from a single thread and having that thread await the completion of
those tasks. There are also utilities for generating the tasks from a slice of data.

This library makes no attempt to ensure fairness or ordering of spawned tasks.

This is a hard fork of the [bevy_tasks](https://crates.io/crates/bevy_tasks) crate, to further
reduce dependencies and simplify the crate.

## Example

```rust
let pool = entangled::TaskPool::default();

let count = std::sync::atomic::AtomicI32::new(0);
let ref_count = &count;

let output = pool.scope(|scope| {
    for _ in 0..100 {
        scope.spawn(async {
            ref_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            1
        });
    }
});

assert_eq!(output.iter().sum::<i32>(), count.load(std::sync::atomic::Ordering::Relaxed));
```

## License

Licensed under MIT.
