// The `sparse_wake_ring` workload from benches/scheduler.rs (19b484b), run once per ring size,
// printing how many rounds and polls it takes. `Link`, `Ring` and the spawn loop are copied
// from the bench unchanged.
use scope::{Cx, Scope, ScopedTask, SchedulerConfig, TaskId, TaskPoll, WakeOrdering, WakeReason};

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

fn config(ordering: WakeOrdering) -> SchedulerConfig {
    SchedulerConfig { wake_ordering: ordering, ..SchedulerConfig::default() }
}

fn main() {
    for links in [10_u32, 100, 1000] {
        let hops = 2000_u64;
        let mut ring = Ring { hops };
        let mut s = Scope::with_config(&mut ring, config(WakeOrdering::BiasedQoSThenAge));
        for i in 0..links {
            s.spawn_named("link", Link { next: TaskId::new((i + 1) % links, 0) });
        }
        let report = s.run();
        println!("links={links} rounds={} polls={} polls_per_round={}", report.rounds, report.polls, report.polls / report.rounds);
    }
}
