use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use futures_lite::future;

use entangled::ThreadPoolDescriptor;

// These benchmarks are a direct copy of the benchmarks `switchyard` provides [MIT/Apache 2.0/ZLIB].

pub fn wide(c: &mut Criterion) {
    let mut group = c.benchmark_group("wide");

    let count = 100_000;

    let future_creation = || {
        let mut array = Vec::with_capacity(count);
        for _ in 0..count {
            array.push(async move { 2 * 2 });
        }
        array
    };

    group.throughput(Throughput::Elements(count as u64));

    group.bench_function("async-std", |b| {
        b.iter_batched(
            future_creation,
            |input| {
                let handle_vec: Vec<_> = input.into_iter().map(async_std::task::spawn).collect();
                future::block_on(async move {
                    for handle in handle_vec {
                        handle.await;
                    }
                })
            },
            BatchSize::PerIteration,
        )
    });

    let yard = switchyard::Switchyard::new(
        1,
        switchyard::threads::single_pool_one_to_one(
            switchyard::threads::thread_info(),
            Some("switchyard"),
        ),
        || (),
    )
    .unwrap();

    group.bench_function("switchyard", |b| {
        b.iter_batched(
            future_creation,
            |input| {
                let handle_vec: Vec<_> =
                    input.into_iter().map(|fut| yard.spawn(0, 0, fut)).collect();
                future::block_on(async move {
                    for handle in handle_vec {
                        handle.await;
                    }
                })
            },
            BatchSize::PerIteration,
        )
    });

    let task_pool = entangled::ThreadPool::new(ThreadPoolDescriptor {
        num_threads: num_cpus::get(),
        ..Default::default()
    })
    .unwrap();

    group.bench_function("entangled", |b| {
        b.iter_batched(
            future_creation,
            |input| {
                let handle_vec: Vec<_> =
                    input.into_iter().map(|fut| task_pool.spawn(fut)).collect();
                future::block_on(async move {
                    for handle in handle_vec {
                        handle.await;
                    }
                })
            },
            BatchSize::PerIteration,
        )
    });

    group.finish();
}

pub fn chain(c: &mut Criterion) {
    let mut group = c.benchmark_group("chain");

    let count = 1_000;

    let (sender, receiver) = async_channel::unbounded();

    group.throughput(Throughput::Elements(count as u64));

    group.bench_function("async-std", |b| {
        b.iter_batched(
            || {
                let receiver = receiver.clone();
                let mut head = async_std::task::spawn(async move {
                    receiver.recv().await.unwrap();
                });
                for _ in 0..count {
                    let old_head = head;
                    head = async_std::task::spawn(async move {
                        old_head.await;
                    });
                }
                head
            },
            |input| {
                sender.try_send(()).unwrap();
                future::block_on(input)
            },
            BatchSize::PerIteration,
        )
    });

    let yard = switchyard::Switchyard::new(
        1,
        switchyard::threads::single_pool_one_to_one(
            switchyard::threads::thread_info(),
            Some("switchyard"),
        ),
        || (),
    )
    .unwrap();

    group.bench_function("switchyard", |b| {
        b.iter_batched(
            || {
                let receiver = receiver.clone();
                let mut head = yard.spawn(0, 0, async move {
                    receiver.recv().await.unwrap();
                });
                for _ in 0..count {
                    let old_head = head;
                    head = yard.spawn(0, 0, async move {
                        old_head.await;
                    });
                }
                head
            },
            |input| {
                sender.try_send(()).unwrap();
                future::block_on(input)
            },
            BatchSize::PerIteration,
        )
    });

    let thread_pool = entangled::ThreadPool::new(entangled::ThreadPoolDescriptor {
        num_threads: num_cpus::get(),
        ..Default::default()
    })
    .unwrap();

    group.bench_function("entangled", |b| {
        b.iter_batched(
            || {
                let receiver = receiver.clone();
                let mut head = thread_pool.spawn(async move {
                    receiver.recv().await.unwrap();
                });
                for _ in 0..count {
                    let old_head = head;
                    head = thread_pool.spawn(async move {
                        old_head.await;
                    });
                }
                head
            },
            |input| {
                sender.try_send(()).unwrap();
                future::block_on(input)
            },
            BatchSize::PerIteration,
        )
    });

    group.finish();
}

criterion_group!(benches, wide, chain);
criterion_main!(benches);
