// A legal borrow with the closure argument typed. Compiles and runs.
use scope::{run_scoped, Cx, FnTask, TaskPoll};

fn main() {
    let mut env = ();
    let outside = String::from("outlives the scope");
    let r = &outside;
    let report = run_scoped(&mut env, |s| {
        s.spawn_named(
            "ok",
            FnTask(move |_cx: &mut Cx<'_, '_, ()>| {
                let _ = r.len();
                TaskPoll::Ready
            }),
        );
    });
    println!("completed={}", report.completed);
}
