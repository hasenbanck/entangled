use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

fn rnd(c: &mut Criterion) {
    for thread_num in [2usize, 4, 8, 16].iter() {
        let mut group = c.benchmark_group(&format!("scoped spawn - {} threads", thread_num));

        for size in [2usize, 4, 8, 16, 32].iter() {
            group.throughput(Throughput::Elements(*size as u64));

            let thread_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(*thread_num)
            .build()
            .unwrap();
            group.bench_function(BenchmarkId::new("rayon", *size), |b| {
                b.iter(|| {
                    thread_pool.scope(|scope| {
                        (0..*size).for_each(|_| {
                            scope.spawn(|_| {
                                black_box(1);
                            });
                        });
                    });
                })
            });

            let task_pool = entangled::TaskPool::new(&entangled::TaskPoolDescriptor {
                num_threads: *thread_num,
                ..Default::default()
            });
            group.bench_function(BenchmarkId::new("entangled", *size), |b| {
                b.iter(|| {
                    task_pool.scope(|scope| {
                        (0..*size).for_each(|_| {
                            scope.spawn(async {
                                black_box(1);
                            });
                        });
                    });
                })
            });
        }
        group.finish();
    }
}

criterion_group!(benches, rnd,);
criterion_main!(benches);
