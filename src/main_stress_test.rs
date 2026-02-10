// examples/stress_hypergraph.rs
//
// Stress goals:
// - Massive fanout (hundreds/thousands of leaves)
// - Repeated fan-in reductions (join_all -> map_join -> join_all -> ...)
// - Tons of wake-heavy polling (Burn + CountTo tasks)
// - select2 storms to force cancellation/loser paths internally
// - Deep dependency chain to stress orchestration bookkeeping
//
// Tune with env vars (optional):
//   LEAVES=800 RACES=400 DEPTH=180 BATCH=8 cargo run --example stress_hypergraph
//
// NOTE: This leaks task names (Box::leak) because your API uses &'static str.
// For a stress test, that's fine.

use std::{pin::Pin, time::Duration};

use scope::{scoped, Cx, JoinError, JoinHandle, LogTracer, Scope, ScopedTask, TaskPoll};

type AppResult<T> = Result<T, &'static str>;

#[derive(Default, Debug)]
struct Resources {
    x: u32,
    polls: u64,
    checksum: u64,
}

// --- Existing style task (wake-heavy) ---
struct CountTo(u32);
impl<'env> ScopedTask<'env, Resources> for CountTo {
    fn poll(self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, Resources>) -> TaskPoll {
        cx.resources.polls += 1;

        if cx.resources.x >= self.0 {
            return TaskPoll::Ready;
        }
        cx.resources.x = cx.resources.x.wrapping_add(1);
        cx.wake_self();
        TaskPoll::Pending
    }
}

// --- Pure CPU burn (wake-heavy), doesn’t need to return values directly ---
// We intentionally do a bunch of tiny operations to create many polls.
struct Burn {
    chunks_left: u32,
    iters_per_poll: u32,
    state: u64,
    id: u32,
    // Optional: enable panic testing behind a feature to avoid accidental crashes if your runtime
    // doesn’t catch panics.
    #[allow(dead_code)]
    panic_when_chunks_left: Option<u32>,
}
impl<'env> ScopedTask<'env, Resources> for Burn {
    fn poll(mut self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, Resources>) -> TaskPoll {
        cx.resources.polls += 1;

        if self.chunks_left == 0 {
            // Commit something deterministic-ish into shared resources to increase contention.
            cx.resources.checksum ^= mix64(self.state ^ (self.id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            return TaskPoll::Ready;
        }

        // Tiny PRNG-ish mixing loop.
        let mut s = self.state;
        for _ in 0..self.iters_per_poll {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s = s.wrapping_mul(0xD6E8_FEB8_6659_FD93);
        }
        self.state = s;
        self.chunks_left -= 1;

        // Optional panic test:
        #[cfg(feature = "panic_test")]
        if self.panic_when_chunks_left == Some(self.chunks_left) {
            panic!("intentional stress panic in Burn(id={})", self.id);
        }

        cx.wake_self();
        TaskPoll::Pending
    }
}

// --- Small deterministic PRNG to vary workload shapes without pulling rand ---
#[derive(Clone, Copy)]
struct XorShift64 {
    s: u64,
}
impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self { s: seed.max(1) }
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.s;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.s = x;
        x
    }
    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }
}

fn mix64(x: u64) -> u64 {
    // Simple reversible-ish avalanche
    let mut v = x;
    v ^= v >> 33;
    v = v.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    v ^= v >> 33;
    v = v.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    v ^= v >> 33;
    v
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}
fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

// Your API wants &'static str, so we leak names for the stress test.
fn name(prefix: &str, a: usize, b: usize) -> &'static str {
    Box::leak(format!("{prefix}_{a}_{b}").into_boxed_str())
}

// Leaf generator: returns JoinHandle<AppResult<u64>> (so join_all is homogeneous)
fn make_leaf(
    scope: &mut Scope<'_, Resources>,
    rng: &mut XorShift64,
    i: usize,
) -> JoinHandle<AppResult<u64>> {
    match (rng.next_u32() % 3) as u8 {
        // Sleep leaf
        0 => {
            let ms = (rng.next_u32() % 25 + 1) as u64;
            let t = scope.sleep(name("leaf_sleep_timer", i, 0), Duration::from_millis(ms));
            scope.spawn_map_join(name("leaf_sleep_map", i, 0), t, move |_, _cx| {
                // return a value, sometimes "soft-fail"
                if i % 251 == 0 {
                    Err("sleep leaf injected error")
                } else {
                    Ok::<u64, &'static str>((i as u64) ^ (ms << 32))
                }
            })
        }

        // Burn leaf (poll-heavy CPU)
        1 => {
            let chunks = (rng.next_u32() % 40 + 25) as u32;
            let burn = Burn {
                chunks_left: chunks,
                iters_per_poll: 300,
                state: rng.next_u64() ^ (i as u64),
                id: i as u32,
                panic_when_chunks_left: if i % 997 == 0 { Some(chunks / 2) } else { None },
            };
            scope.spawn_with_result(name("leaf_burn", i, 0), burn, move |_t, _res| {
                // inject some failures to exercise error propagation
                if i % 149 == 0 {
                    Err("burn leaf sabotage")
                } else {
                    Ok::<u64, &'static str>(0xBADC0FFE_u64 ^ (i as u64))
                }
            })
        }

        // CountTo leaf (contention on Resources.x)
        _ => {
            let target = (rng.next_u32() % 80 + 10) as u32;
            scope.spawn_with_result(name("leaf_count", i, 0), CountTo(target), move |_t, res| {
                // occasional error checkpoints based on evolving shared state
                if (res.x % 127 == 0) && (i % 17 == 0) {
                    Err("count leaf hit unlucky checkpoint")
                } else {
                    Ok::<u64, &'static str>(res.x as u64 ^ (i as u64).wrapping_mul(31))
                }
            })
        }
    }
}

// Reduce a group via join_all -> map_join => one handle
fn reduce_group(
    scope: &mut Scope<'_, Resources>,
    level: usize,
    group_index: usize,
    group: Vec<JoinHandle<AppResult<u64>>>,
) -> JoinHandle<AppResult<u64>> {
    let all = scope.spawn_join_all(name("reduce_join_all", level, group_index), group);

    scope.spawn_map_join(name("reduce_map", level, group_index), all, move |res, _cx| {
        // res: Result<Vec<Result<AppResult<u64>, JoinError>>, JoinError>
        let vec = match res {
            Ok(v) => v,
            Err(JoinError::Panicked) => return Err("reduce_group: join_all panicked"),
            Err(JoinError::Cancelled(_)) => return Err("reduce_group: join_all cancelled"),
        };

        // Fold everything into a single checksum-ish number.
        let mut acc = mix64((level as u64) ^ ((group_index as u64) << 32) ^ 0xA5A5_A5A5_A5A5_A5A5);

        for item in vec {
            let app = match item {
                Ok(app) => app,
                Err(JoinError::Panicked) => return Err("reduce_group: child panicked"),
                Err(JoinError::Cancelled(_)) => return Err("reduce_group: child cancelled"),
            };

            match app {
                Ok(v) => acc = mix64(acc ^ v),
                Err(_e) => acc = mix64(acc ^ 0xDEAD_DEAD_DEAD_DEAD),
            }
        }

        Ok::<u64, &'static str>(acc)
    })
}

// Multi-round reduction: repeatedly batch into groups (BATCH) until one handle remains.
fn reduce_tree(
    scope: &mut Scope<'_, Resources>,
    mut handles: Vec<JoinHandle<AppResult<u64>>>,
    batch: usize,
) -> JoinHandle<AppResult<u64>> {
    let mut level = 0usize;

    while handles.len() > 1 {
        let mut next = Vec::new();
        let mut gi = 0usize;

        while !handles.is_empty() {
            let take = batch.min(handles.len());
            let group: Vec<_> = handles.drain(0..take).collect();
            next.push(reduce_group(scope, level, gi, group));
            gi += 1;
        }

        handles = next;
        level += 1;
    }

    handles.pop().expect("reduce_tree called with empty handles")
}

// Large select2 storm: create many independent races, then reduce them.
fn select2_storm(
    scope: &mut Scope<'_, Resources>,
    rng: &mut XorShift64,
    races: usize,
    batch: usize,
) -> JoinHandle<AppResult<u64>> {
    let mut outs = Vec::with_capacity(races);

    for i in 0..races {
        let fast_ms = (rng.next_u32() % 4 + 1) as u64;
        let slow_ms = (rng.next_u32() % 50 + 10) as u64;

        let fast = scope.sleep(name("race_fast", i, 0), Duration::from_millis(fast_ms));
        let slow = scope.sleep(name("race_slow", i, 0), Duration::from_millis(slow_ms));

        let sel = scope.spawn_select2(name("race_sel", i, 0), fast, slow);

        // We don’t depend on the concrete select2 output type — just treat success vs join error.
        let mapped = scope.spawn_map_join(name("race_map", i, 0), sel, move |res, _cx| {
            match res {
                Ok(_whatever) => {
                    if i % 313 == 0 {
                        Err("race injected failure")
                    } else {
                        Ok::<u64, &'static str>(mix64((i as u64) ^ (fast_ms << 16) ^ (slow_ms << 32)))
                    }
                }
                Err(JoinError::Panicked) => Err("race: select2 panicked"),
                Err(JoinError::Cancelled(_)) => Err("race: select2 cancelled"),
            }
        });

        outs.push(mapped);
    }

    reduce_tree(scope, outs, batch)
}

// Deep dependency chain (no handle sharing): build recursively (depth N), then map results upward.
fn deep_chain(scope: &mut Scope<'_, Resources>, depth: u32) -> JoinHandle<AppResult<u64>> {
    fn rec(scope: &mut Scope<'_, Resources>, d: u32) -> JoinHandle<AppResult<u64>> {
        let nm = Box::leak(format!("chain_{d}").into_boxed_str());

        if d == 0 {
            // base leaf does real polling work
            let burn = Burn {
                chunks_left: 80,
                iters_per_poll: 400,
                state: 0x1234_5678_9ABC_DEF0,
                id: 0,
                panic_when_chunks_left: None,
            };
            return scope.spawn_with_result(nm, burn, |_t, _res| Ok::<u64, &'static str>(mix64(0xC0FFEE)));
        }

        // child
        let child = rec(scope, d - 1);

        // add a timer at each level to ensure timer + poll interplay
        let t = scope.sleep(Box::leak(format!("chain_sleep_{d}").into_boxed_str()), Duration::from_millis((d % 7) as u64));
        let t_mapped = scope.spawn_map_join(Box::leak(format!("chain_sleep_map_{d}").into_boxed_str()), t, move |_, _| {
            Ok::<u64, &'static str>(mix64(d as u64 ^ 0x55AA_55AA))
        });

        // join child + timer, then fold
        let both = scope.spawn_join_all(Box::leak(format!("chain_join_{d}").into_boxed_str()), vec![child, t_mapped]);

        scope.spawn_map_join(Box::leak(format!("chain_fold_{d}").into_boxed_str()), both, move |res, _cx| {
            let vec = match res {
                Ok(v) => v,
                Err(JoinError::Panicked) => return Err("chain: join panicked"),
                Err(JoinError::Cancelled(_)) => return Err("chain: join cancelled"),
            };

            let mut acc = mix64(d as u64 ^ 0xAAAA_BBBB_CCCC_DDDD);

            for item in vec {
                let app = match item {
                    Ok(app) => app,
                    Err(JoinError::Panicked) => return Err("chain: child panicked"),
                    Err(JoinError::Cancelled(_)) => return Err("chain: child cancelled"),
                };
                match app {
                    Ok(v) => acc = mix64(acc ^ v),
                    Err(_) => acc = mix64(acc ^ 0x1111_2222_3333_4444),
                }
            }

            // occasionally inject a failure near the top
            if d % 97 == 0 {
                Err("chain injected failure")
            } else {
                Ok::<u64, &'static str>(acc)
            }
        })
    }

    rec(scope, depth)
}

fn main() {
    let start = std::time::Instant::now();

    let leaves = env_usize("LEAVES", 600);
    let races = env_usize("RACES", 250);
    let depth = env_u32("DEPTH", 140);
    let batch = env_usize("BATCH", 8).max(2);

    let mut resources = Resources::default();

    scoped(&mut resources, |scope| {
        scope.set_tracer(Box::new(LogTracer));

        // A long-running “monitor” to force extra contention & wakeups.
        // It completes eventually (so it doesn’t hang the run).
        let monitor = scope.spawn_with_result("monitor", CountTo(10_000), |_t, res| {
            // Note: this closure only runs once the task completes.
            println!("monitor finished: x={}", res.x);
            Ok::<u64, &'static str>(res.x as u64)
        });

        let mut rng = XorShift64::new(0xDEAD_BEEF_F00D_BA5E);

        // 1) Big fanout
        let mut leaf_handles = Vec::with_capacity(leaves);
        for i in 0..leaves {
            leaf_handles.push(make_leaf(scope, &mut rng, i));
        }

        // 2) Fan-in reduction tree
        let reduced = reduce_tree(scope, leaf_handles, batch);

        // 3) select2 storm reduction
        let races_reduced = select2_storm(scope, &mut rng, races, batch);

        // 4) Deep dependency chain
        let chained = deep_chain(scope, depth);

        // 5) Final join + report
        let final_all = scope.spawn_join_all("FINAL_JOIN_ALL", vec![reduced, races_reduced, chained, monitor]);

        let _report = scope.spawn_map_join("REPORT", final_all, |res, cx| {
            println!("\n================ FINAL REPORT ================");
            match res {
                Ok(items) => {
                    println!("FINAL_JOIN_ALL: Ok ({} items)", items.len());
                    for (i, it) in items.into_iter().enumerate() {
                        match it {
                            Ok(app) => match app {
                                Ok(v) => println!("  [{i}] ok: 0x{v:016x}"),
                                Err(e) => println!("  [{i}] app-err: {e}"),
                            },
                            Err(e) => println!("  [{i}] join-err: {e:?}"),
                        }
                    }
                }
                Err(e) => println!("FINAL_JOIN_ALL crashed: {e:?}"),
            }

            println!(
                "RESOURCES: x={} polls={} checksum=0x{:016x}",
                cx.resources.x, cx.resources.polls, cx.resources.checksum
            );
            println!("=============================================\n");
            Ok::<(), &'static str>(())
        });

        scope.run();
    });
    let elapsed = start.elapsed();
    println!("total runtime: {:?}", elapsed);

    let secs = elapsed.as_secs_f64();
    println!("total runtime: {:.3}s", secs);

}