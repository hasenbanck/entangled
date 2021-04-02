# entangled
[![Latest version](https://img.shields.io/crates/v/entangled.svg)](https://crates.io/crates/entangled)
[![Documentation](https://docs.rs/entangled/badge.svg)](https://docs.rs/entangled)
![ZLIB](https://img.shields.io/badge/license-zlib-blue.svg)
![MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![Apache](https://img.shields.io/badge/license-Apache-blue.svg)

Entangled provides a thread pool based on
the [async-executor](https://crates.io/crates/async-executor) crate to spawn async futures on.

It's main selling point is the "scoped spawn" functionality, which is essentially forking tasks from
the main thread, which have access to the stack of the main thread and joins them after they have
completed.

## Example

```rust
let pool = entangled::TaskPool::default ();

let count = std::sync::atomic::AtomicI32::new(0);
let ref_count = & count;

let output = pool.scope( | scope| {
for _ in 0..100 {
scope.spawn(async {
ref_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
1
});
}
});

assert_eq!(output.iter().sum::<i32>(), count.load(std::sync::atomic::Ordering::Relaxed));
```

## Credits

Originally based on [bevy_tasks](https://crates.io/crates/bevy_tasks) crate, but was completely
rewritten with version 1.2

## License

Licensed under Apache-2.0 or MIT or ZLIB.
