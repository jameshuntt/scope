// Two runnable tasks among many parked ones: `idle` tasks park on their first poll; two tasks pass
// 2000 wakes back and forth; when the hops run out the last one wakes every parked task.
use scope::{Cx, Scope, ScopedTask, TaskId, TaskPoll, WakeReason};
use std::time::Instant;
struct Env { hops: u64, idle: u32 }
struct Parked { polled: bool }
impl<'env> ScopedTask<'env, Env> for Parked {
    fn poll(&mut self, _cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
        if self.polled { return TaskPoll::Ready; }
        self.polled = true;
        TaskPoll::Pending
    }
}
struct Ping { other: TaskId }
impl<'env> ScopedTask<'env, Env> for Ping {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
        if cx.resources.hops == 0 {
            for i in 0..cx.resources.idle { cx.wake_with(TaskId::new(i, 0), WakeReason::Manual); }
            cx.wake_with(self.other, WakeReason::Manual);
            return TaskPoll::Ready;
        }
        cx.resources.hops -= 1;
        cx.wake_with(self.other, WakeReason::Manual);
        TaskPoll::Pending
    }
}
fn main() {
    for idle in [10u32, 100, 1000] {
        let mut times = Vec::new();
        let mut last = (0, 0, 0);
        for _ in 0..101 {
            let start = Instant::now();
            let mut env = Env { hops: 2000, idle };
            let r = {
                let mut s = Scope::new(&mut env);
                for _ in 0..idle { s.spawn_named("parked", Parked { polled: false }); }
                s.spawn_named("ping", Ping { other: TaskId::new(idle + 1, 0) });
                s.spawn_named("pong", Ping { other: TaskId::new(idle, 0) });
                s.run()
            };
            times.push(start.elapsed().as_nanos());
            last = (r.rounds, r.polls, r.completed);
        }
        times.sort();
        println!("idle={idle} rounds={} polls={} completed={} median_us={:.1}", last.0, last.1, last.2, times[50] as f64 / 1000.0);
    }
}
