// scope 0.2.2 declares rust-version 1.80. Build this on 1.80 and on 1.85.
fn main() {
    let mut env = 0_u64;
    let report = scope::run_scoped(&mut env, |_s| {});
    println!("scope 0.2.2 built; completed={}", report.completed);
}
