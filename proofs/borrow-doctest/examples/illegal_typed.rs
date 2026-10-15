// The illegal borrow with the closure argument typed. Does not compile: one error, E0597.
use scope::{run_scoped, Cx, FnTask, TaskPoll};

fn main() {
    let mut env = ();
    run_scoped(&mut env, |s| {
        let local = String::from("dies when this closure returns");
        let r = &local;
        s.spawn_named(
            "bad",
            FnTask(move |_cx: &mut Cx<'_, '_, ()>| {
                let _ = r.len();
                TaskPoll::Ready
            }),
        );
    });
}
