// src/trace.rs (UPDATED - adds poll sequence hook)
use crate::task::{TaskId, TaskState};

use std::time::Duration;
use crate::cancel_reason::CancelReason;

pub trait Tracer {
    fn on_cancel_request(&mut self, _task: TaskId, _reason: CancelReason, _tick_budget: Option<u64>, _time_budget: Option<Duration>) {}
    fn on_cancel_hard_stop(&mut self, _task: TaskId, _reason: CancelReason) {}

    // fn on_cancel_request(&mut self, _task: TaskId, _tick_budget: Option<u64>, _time_budget: Option<Duration>) {}
    // fn on_cancel_hard_stop(&mut self, _task: TaskId) {}

    fn on_spawn(&mut self, _id: TaskId, _name: &str) {}
    fn on_wake(&mut self, _from: TaskId, _to: TaskId) {}

    fn on_subscribe_finish(&mut self, _waiter: TaskId, _target: TaskId) {}

    fn on_poll_seq(&mut self, _seq: u64, _id: TaskId, _name: &str) {}
    fn on_poll_begin(&mut self, _id: TaskId, _name: &str) {}
    fn on_poll_end(&mut self, _id: TaskId, _name: &str) {}

    fn on_finish(&mut self, _id: TaskId, _name: &str, _state: TaskState) {}
    fn on_defer_spawn(&mut self, _parent: TaskId, _child_name: &str) {}

    fn on_io_wait_readable(&mut self, _task: TaskId, _token: usize) {}
    fn on_io_notify_readable(&mut self, _token: usize, _woken: &[TaskId]) {}

    fn on_sleep_until(&mut self, _task: TaskId, _deadline_micros_from_now: u128) {}
    fn on_timers_fired(&mut self, _count: usize) {}
}

pub struct LogTracer;
impl Tracer for LogTracer {
    fn on_spawn(&mut self, id: TaskId, name: &str) {
        eprintln!("[spawn] {id} {name}");
    }
    fn on_wake(&mut self, from: TaskId, to: TaskId) {
        eprintln!("[wake] {from} -> {to}");
    }
    fn on_subscribe_finish(&mut self, waiter: TaskId, target: TaskId) {
        eprintln!("[join] {waiter} waits {target}");
    }
    fn on_poll_seq(&mut self, seq: u64, id: TaskId, name: &str) {
        eprintln!("[poll#] {seq}  {id} {name}");
    }
    fn on_finish(&mut self, id: TaskId, name: &str, state: TaskState) {
        eprintln!("[done] {id} {name} => {state:?}");
    }
    fn on_defer_spawn(&mut self, parent: TaskId, child: &str) {
        eprintln!("[spawn_later] parent={parent} child={child}");
    }
    fn on_sleep_until(&mut self, task: TaskId, micros: u128) {
        eprintln!("[sleep] task {task} until +{micros}us");
    }
    fn on_timers_fired(&mut self, count: usize) {
        eprintln!("[timers] fired {count}");
    }
}

// DotTracer unchanged from prior version (keep your existing one if you want)
pub struct DotTracer {
    edges: Vec<(TaskId, TaskId)>,
    labels: Vec<(TaskId, String)>,
}
impl DotTracer {
    pub fn new() -> Self {
        Self { edges: vec![], labels: vec![] }
    }
    pub fn dot(&self) -> String {
        let mut s = String::from("digraph borrowed_executor {\n");
        for (id, name) in &self.labels {
            s.push_str(&format!("  n{id} [label=\"{id}: {name}\"];\n"));
        }
        for (a, b) in &self.edges {
            s.push_str(&format!("  n{a} -> n{b};\n"));
        }
        s.push_str("}\n");
        s
    }
}
impl Tracer for DotTracer {
    fn on_spawn(&mut self, id: TaskId, name: &str) {
        self.labels.push((id, name.to_string()));
    }
    fn on_wake(&mut self, from: TaskId, to: TaskId) {
        self.edges.push((from, to));
    }
    fn on_subscribe_finish(&mut self, waiter: TaskId, target: TaskId) {
        self.edges.push((target, waiter));
    }
}





/// ✅ stores trace lines in memory (Vec<String>) instead of printing
pub struct BufferTracer {
    lines: Vec<String>,
}
impl BufferTracer {
    pub fn new() -> Self {
        Self { lines: Vec::new() }
    }
    pub fn push(&mut self, s: impl Into<String>) {
        self.lines.push(s.into());
    }
    pub fn lines(&self) -> &[String] {
        &self.lines
    }
    pub fn take(self) -> Vec<String> {
        self.lines
    }
}
impl Tracer for BufferTracer {
    fn on_spawn(&mut self, id: TaskId, name: &str) {
        self.push(format!("[spawn] {id} {name}"));
    }
    fn on_wake(&mut self, from: TaskId, to: TaskId) {
        self.push(format!("[wake] {from} -> {to}"));
    }
    fn on_subscribe_finish(&mut self, waiter: TaskId, target: TaskId) {
        self.push(format!("[join] {waiter} waits {target}"));
    }
    fn on_poll_seq(&mut self, seq: u64, id: TaskId, name: &str) {
        self.push(format!("[poll#] {seq} {id} {name}"));
    }
    fn on_finish(&mut self, id: TaskId, name: &str, state: TaskState) {
        self.push(format!("[done] {id} {name} => {state:?}"));
    }
    fn on_timers_fired(&mut self, count: usize) {
        self.push(format!("[timers] fired {count}"));
    }
    

    fn on_cancel_request(&mut self, task: TaskId, reason: CancelReason, tick_budget: Option<u64>, time_budget: Option<std::time::Duration>) {
        self.push(format!(
            "[cancel] task={task} reason={reason:?} tick={:?} time_ms={:?}",
            tick_budget,
            time_budget.map(|d| d.as_millis())
        ));
    }

    fn on_cancel_hard_stop(&mut self, task: TaskId, reason: CancelReason) {
        self.push(format!("[cancel_now] task={task} reason={reason:?}"));
    }
}









// use crate::task::{TaskId, TaskState};
// 
// pub trait Tracer {
//     fn on_spawn(&mut self, _id: TaskId, _name: &str) {}
//     fn on_wake(&mut self, _from: TaskId, _to: TaskId) {}
//     fn on_subscribe_finish(&mut self, _waiter: TaskId, _target: TaskId) {}
//     fn on_poll_begin(&mut self, _id: TaskId, _name: &str) {}
//     fn on_poll_end(&mut self, _id: TaskId, _name: &str) {}
//     fn on_finish(&mut self, _id: TaskId, _name: &str, _state: TaskState) {}
//     fn on_defer_spawn(&mut self, _parent: TaskId, _child_name: &str) {}
//     fn on_io_wait_readable(&mut self, _task: TaskId, _token: usize) {}
//     fn on_io_notify_readable(&mut self, _token: usize, _woken: &[TaskId]) {}
// }
// 
// pub struct LogTracer;
// impl Tracer for LogTracer {
//     fn on_spawn(&mut self, id: TaskId, name: &str) {
//         eprintln!("[spawn] {id} {name}");
//     }
//     fn on_wake(&mut self, from: TaskId, to: TaskId) {
//         eprintln!("[wake] {from} -> {to}");
//     }
//     fn on_subscribe_finish(&mut self, waiter: TaskId, target: TaskId) {
//         eprintln!("[join] {waiter} waits {target}");
//     }
//     fn on_poll_begin(&mut self, id: TaskId, name: &str) {
//         eprintln!("[poll+] {id} {name}");
//     }
//     fn on_poll_end(&mut self, id: TaskId, name: &str) {
//         eprintln!("[poll-] {id} {name}");
//     }
//     fn on_finish(&mut self, id: TaskId, name: &str, state: TaskState) {
//         eprintln!("[done] {id} {name} => {state:?}");
//     }
//     fn on_io_wait_readable(&mut self, task: TaskId, token: usize) {
//         eprintln!("[io] task {task} waits readable({token})");
//     }
//     fn on_io_notify_readable(&mut self, token: usize, woken: &[TaskId]) {
//         eprintln!("[io] notify readable({token}) wakes {woken:?}");
//     }
// }
// 
// pub struct DotTracer {
//     edges: Vec<(TaskId, TaskId)>,
//     labels: Vec<(TaskId, String)>,
// }
// impl DotTracer {
//     pub fn new() -> Self {
//         Self { edges: vec![], labels: vec![] }
//     }
//     pub fn dot(&self) -> String {
//         let mut s = String::from("digraph borrowed_executor {\n");
//         for (id, name) in &self.labels {
//             s.push_str(&format!("  n{id} [label=\"{id}: {name}\"];\n"));
//         }
//         for (a, b) in &self.edges {
//             s.push_str(&format!("  n{a} -> n{b};\n"));
//         }
//         s.push_str("}\n");
//         s
//     }
// }
// impl Tracer for DotTracer {
//     fn on_spawn(&mut self, id: TaskId, name: &str) {
//         self.labels.push((id, name.to_string()));
//     }
//     fn on_wake(&mut self, from: TaskId, to: TaskId) {
//         self.edges.push((from, to));
//     }
//     fn on_subscribe_finish(&mut self, waiter: TaskId, target: TaskId) {
//         self.edges.push((target, waiter)); // edge from producer -> waiter
//     }
// }
// 