//! The scheduler's own cost, per workload. Each iteration builds a scope over a borrowed
//! environment, spawns the tasks and runs to completion, so the numbers include spawn and finish.
//!
//!   cargo bench --bench scheduler

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use scope::{Cx, Mailbox, QoSClass, Scope, ScopedTask, SchedulerConfig, TaskId, TaskPoll, WakeOrdering, WakeReason};

/// Yields `left` times, adding to the shared counter each poll.
struct Yielder {
    left: u32,
}

impl<'env> ScopedTask<'env, u64> for Yielder {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, u64>) -> TaskPoll {
        *cx.resources += 1;
        if self.left == 0 {
            return TaskPoll::Ready;
        }
        self.left -= 1;
        cx.yield_now()
    }
}

/// Waits to be woken; wakes the next link. Only one link is runnable at a time, so every round
/// has one runnable task among many idle ones.
struct Link {
    next: TaskId,
}

struct Ring {
    hops: u64,
}

impl<'env> ScopedTask<'env, Ring> for Link {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Ring>) -> TaskPoll {
        if cx.resources.hops == 0 {
            cx.wake_with(self.next, WakeReason::Manual);
            return TaskPoll::Ready;
        }
        cx.resources.hops -= 1;
        cx.wake_with(self.next, WakeReason::Manual);
        TaskPoll::Pending
    }
}

/// Spawns `children` tasks that each finish on their first poll.
struct Spawner {
    children: u32,
    spawned: bool,
}

struct Done;

impl<'env> ScopedTask<'env, u64> for Done {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, u64>) -> TaskPoll {
        *cx.resources += 1;
        TaskPoll::Ready
    }
}

impl<'env> ScopedTask<'env, u64> for Spawner {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, u64>) -> TaskPoll {
        if self.spawned {
            return TaskPoll::Ready;
        }
        self.spawned = true;
        for _ in 0..self.children {
            cx.spawn_later_named("child", Done);
        }
        cx.yield_now()
    }
}

/// Drains its mailbox until it has seen `expect` messages.
struct Inbox {
    mailbox: Mailbox<u64>,
    expect: u64,
    seen: u64,
}

impl<'env> ScopedTask<'env, u64> for Inbox {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, u64>) -> TaskPoll {
        while let Some(m) = self.mailbox.try_recv() {
            *cx.resources += m;
            self.seen += 1;
        }
        if self.seen >= self.expect {
            TaskPoll::Ready
        } else {
            TaskPoll::Pending
        }
    }
}

const QOS: [QoSClass; 5] =
    [QoSClass::Background, QoSClass::Batch, QoSClass::Standard, QoSClass::Interactive, QoSClass::Realtime];

fn config(ordering: WakeOrdering) -> SchedulerConfig {
    SchedulerConfig { wake_ordering: ordering, ..SchedulerConfig::default() }
}

fn yield_storm(c: &mut Criterion) {
    let mut group = c.benchmark_group("yield_storm");
    for ordering in [WakeOrdering::BiasedQoSThenAge, WakeOrdering::RoundRobin, WakeOrdering::Fifo] {
        for tasks in [10_u32, 100, 1000] {
            let polls = u64::from(tasks) * 21;
            group.throughput(Throughput::Elements(polls));
            group.bench_with_input(BenchmarkId::new(ordering.name(), tasks), &tasks, |b, &tasks| {
                b.iter(|| {
                    let mut counter = 0_u64;
                    let mut s = Scope::with_config(&mut counter, config(ordering));
                    for i in 0..tasks {
                        s.spawn_named_qos("y", QOS[i as usize % 5], Yielder { left: 20 });
                    }
                    let report = s.run();
                    black_box(report.polls);
                });
            });
        }
    }
    group.finish();
}

fn sparse_wake_ring(c: &mut Criterion) {
    let mut group = c.benchmark_group("sparse_wake_ring");
    for links in [10_u32, 100, 1000] {
        let hops = 2000_u64;
        group.throughput(Throughput::Elements(hops));
        group.bench_with_input(BenchmarkId::from_parameter(links), &links, |b, &links| {
            b.iter(|| {
                let mut ring = Ring { hops };
                let mut s = Scope::with_config(&mut ring, config(WakeOrdering::BiasedQoSThenAge));
                for i in 0..links {
                    s.spawn_named("link", Link { next: TaskId::new((i + 1) % links, 0) });
                }
                let report = s.run();
                black_box(report.polls);
            });
        });
    }
    group.finish();
}

fn spawn_churn(c: &mut Criterion) {
    let mut group = c.benchmark_group("spawn_churn");
    for children in [100_u32, 1000, 10_000] {
        group.throughput(Throughput::Elements(u64::from(children)));
        group.bench_with_input(BenchmarkId::from_parameter(children), &children, |b, &children| {
            b.iter(|| {
                let mut counter = 0_u64;
                let mut s = Scope::new(&mut counter);
                s.spawn_named("spawner", Spawner { children, spawned: false });
                black_box(s.run().completed);
            });
        });
    }
    group.finish();
}

fn join_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("join_all");
    for tasks in [10_u32, 100, 1000] {
        group.throughput(Throughput::Elements(u64::from(tasks)));
        group.bench_with_input(BenchmarkId::from_parameter(tasks), &tasks, |b, &tasks| {
            b.iter(|| {
                let mut counter = 0_u64;
                let mut s = Scope::new(&mut counter);
                let handles: Vec<_> =
                    (0..tasks).map(|i| s.spawn_with_result("w", Yielder { left: i % 4 }, |t, _| t.left)).collect();
                let all = s.spawn_join_all("all", handles);
                s.run();
                black_box(all.take());
            });
        });
    }
    group.finish();
}

fn mailbox(c: &mut Criterion) {
    let mut group = c.benchmark_group("mailbox");
    for messages in [100_u64, 10_000] {
        group.throughput(Throughput::Elements(messages));
        group.bench_with_input(BenchmarkId::from_parameter(messages), &messages, |b, &messages| {
            b.iter(|| {
                let mut counter = 0_u64;
                let mut s = Scope::new(&mut counter);
                let (_, tx) = s.spawn_with_mailbox("inbox", |mailbox| Inbox { mailbox, expect: messages, seen: 0 });
                for m in 0..messages {
                    tx.send(m);
                }
                black_box(s.run().polls);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, yield_storm, sparse_wake_ring, spawn_churn, join_all, mailbox);
criterion_main!(benches);
