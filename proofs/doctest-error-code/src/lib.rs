//! Does rustdoc check the error code on a `compile_fail` doc test? The snippet below is the
//! `compile_fail,E0597` example from scope 0.3.1's `run_scoped`, but marked with the wrong code,
//! E0308. Stable `cargo test --doc` passes it. `cargo +nightly test --doc` fails it, because
//! E0308 is not among the errors.

/// The 0.3.1 `run_scoped` example that must not compile, marked with the wrong error code.
///
/// ```compile_fail,E0308
/// use scope::{run_scoped, Cx, FnTask, TaskPoll};
///
/// let mut env = ();
/// run_scoped(&mut env, |s| {
///     let local = String::from("dies when this closure returns");
///     let r = &local; // borrow of closure-local data
///     s.spawn_named(
///         "bad",
///         FnTask(move |_cx: &mut Cx<'_, '_, ()>| {
///             let _ = r.len(); // task would read it after `local` is dropped
///             TaskPoll::Ready
///         }),
///     );
/// }); // ERROR: `local` does not live long enough
/// ```
pub fn wrong_error_code() {}
