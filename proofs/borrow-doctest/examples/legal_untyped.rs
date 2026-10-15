// A legal borrow with the untyped closure. Does not compile: four "not general enough", no E0597.
use scope::{run_scoped, FnTask, TaskPoll};

fn main() {
    let mut env = ();
    let outside = String::from("outlives the scope");
    let r = &outside;
    run_scoped(&mut env, |s| {
        s.spawn_named(
            "ok",
            FnTask(move |_cx| {
                let _ = r.len();
                TaskPoll::Ready
            }),
        );
    });
}
