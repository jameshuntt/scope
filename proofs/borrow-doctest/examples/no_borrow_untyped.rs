// No borrow at all, with the untyped closure. Does not compile: four "not general enough".
use scope::{run_scoped, FnTask, TaskPoll};

fn main() {
    let mut env = ();
    run_scoped(&mut env, |s| {
        s.spawn_named("ok", FnTask(move |_cx| TaskPoll::Ready));
    });
}
