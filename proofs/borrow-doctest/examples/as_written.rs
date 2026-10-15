// The 0.2.2 doc test as a program. Does not compile: E0597 plus four "not general enough".
use scope::{run_scoped, FnTask, TaskPoll};

fn main() {
    let mut env = ();
    run_scoped(&mut env, |s| {
        let local = String::from("dies when this closure returns");
        let r = &local; // borrow of closure-local data
        s.spawn_named(
            "bad",
            FnTask(move |_cx| {
                let _ = r.len(); // task would read it after `local` is dropped
                TaskPoll::Ready
            }),
        );
    }); // ERROR: `local` does not live long enough
}
