//! The `run_scoped` doc test that must not compile, as scope 0.2.2 wrote it, and three variants.
//! They differ only in where the borrowed `String` lives and whether the task closure's argument
//! has a type. Run `cargo test --doc`; all four pass.

/// The 0.2.2 doc test, unchanged (src/scope.rs, lines 140-155 of the 0.2.2 package).
///
/// ```compile_fail
/// use scope::{run_scoped, FnTask, TaskPoll};
///
/// let mut env = ();
/// run_scoped(&mut env, |s| {
///     let local = String::from("dies when this closure returns");
///     let r = &local; // borrow of closure-local data
///     s.spawn_named(
///         "bad",
///         FnTask(move |_cx| {
///             let _ = r.len(); // task would read it after `local` is dropped
///             TaskPoll::Ready
///         }),
///     );
/// }); // ERROR: `local` does not live long enough
/// ```
pub fn as_written() {}

/// The same code with the `String` moved out of the scope, so the borrow is legal.
/// Still marked `compile_fail`, and it still passes.
///
/// ```compile_fail
/// use scope::{run_scoped, FnTask, TaskPoll};
///
/// let mut env = ();
/// let outside = String::from("outlives the scope");
/// let r = &outside;
/// run_scoped(&mut env, |s| {
///     s.spawn_named(
///         "ok",
///         FnTask(move |_cx| {
///             let _ = r.len();
///             TaskPoll::Ready
///         }),
///     );
/// });
/// ```
pub fn legal_borrow_untyped_closure() {}

/// The legal borrow with the closure argument typed. It compiles and runs.
///
/// ```
/// use scope::{run_scoped, Cx, FnTask, TaskPoll};
///
/// let mut env = ();
/// let outside = String::from("outlives the scope");
/// let r = &outside;
/// let report = run_scoped(&mut env, |s| {
///     s.spawn_named(
///         "ok",
///         FnTask(move |_cx: &mut Cx<'_, '_, ()>| {
///             let _ = r.len();
///             TaskPoll::Ready
///         }),
///     );
/// });
/// assert_eq!(report.completed, 1);
/// ```
pub fn legal_borrow_typed_closure() {}

/// The illegal borrow with the closure argument typed. Rejected with E0597.
///
/// ```compile_fail,E0597
/// use scope::{run_scoped, Cx, FnTask, TaskPoll};
///
/// let mut env = ();
/// run_scoped(&mut env, |s| {
///     let local = String::from("dies when this closure returns");
///     let r = &local;
///     s.spawn_named(
///         "bad",
///         FnTask(move |_cx: &mut Cx<'_, '_, ()>| {
///             let _ = r.len();
///             TaskPoll::Ready
///         }),
///     );
/// });
/// ```
pub fn illegal_borrow_typed_closure() {}
