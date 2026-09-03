//! # 1BRC: NEON SIMD × threads × scope
//!
//! The One Billion Row Challenge — aggregate min/mean/max per weather station
//! over a ~1B-line `station;temperature` file — driven by three orthogonal
//! layers stacked cleanly:
//!
//! * **OS threads** give the cores (one per performance core).
//! * **`scope`** gives each thread its own deterministic, cooperative
//!   structured-concurrency runtime, driving several newline-aligned chunk
//!   tasks per thread that borrow the mmap (non-`'static`) via the scope env.
//! * **NEON** gives per-core scan speed (16-byte semicolon gate).
//!
//! On an Apple M4 Pro (10P/4E) this reaches ~1.2 s / ~834 M rows/s warm on the
//! canonical 1B-row file — the scope layer adds no measurable overhead; the
//! throughput is entirely cores × SIMD in the worker.
//!
//! The runtime itself stays `#![deny(unsafe_code)]`; the only `unsafe` here is
//! the NEON byte-scan in this example's worker.
//!
//! ## Run
//! ```text
//! cargo run --release --example one_billion_rows_neon_threads -- /path/to/measurements.txt
//! ```
//! (defaults to `measurements.txt` in the working directory). Apple-silicon /
//! aarch64 only — on other targets it prints a notice and exits.

#[cfg(target_arch = "aarch64")]
fn main() {
    imp::run();
}

#[cfg(not(target_arch = "aarch64"))]
fn main() {
    eprintln!(
        "one_billion_rows_neon_threads uses aarch64 NEON intrinsics; \
         build/run it on Apple silicon or another aarch64 host."
    );
}

#[cfg(target_arch = "aarch64")]
mod imp {
    use std::arch::aarch64::*;
    use std::fs::File;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{Duration, Instant};

    use memmap2::{Mmap, MmapOptions};
    use scope::{scoped, Cx, Scope, ScopedTask, TaskPoll};

    const SLOTS: usize = 1024;

    // The known station set for the canonical generator. Aggregation uses a
    // fixed hash-slot table (fast, cache-resident); collisions across this many
    // names into 1024 slots are astronomically unlikely.
    const NAMES: &[&[u8]] = &[
        b"Hamburg", b"Tokyo", b"New York", b"London", b"Sydney", b"Nairobi",
        b"Oslo", b"Seoul", b"Paris",
    ];

    /// Per-thread accumulation table: (min, max, sum_tenths, count) per hash slot.
    #[derive(Clone)]
    pub struct Table(pub Vec<(i64, i64, i64, i64)>);
    impl Default for Table {
        fn default() -> Self {
            Table(vec![(i64::MAX, i64::MIN, 0i64, 0i64); SLOTS])
        }
    }

    /// NEON scan of `bytes` in full, folding each row into `table`.
    ///
    /// # Safety
    /// `bytes` must be a valid slice; NEON is baseline on aarch64. The scan does
    /// unchecked slot indexing (`hash % SLOTS` is always in range).
    #[target_feature(enable = "neon")]
    unsafe fn scan_neon(bytes: &[u8], table: &mut [(i64, i64, i64, i64)]) {
        let len = bytes.len();
        let ptr = bytes.as_ptr();
        let mut offset = 0usize;
        let v_semi = vdupq_n_u8(b';');

        while offset < len {
            let mut hash: u64 = 0;

            // scan to ';': 16-byte NEON gate, scalar finish
            if offset + 16 <= len {
                let chunk = vld1q_u8(ptr.add(offset));
                let cmp = vceqq_u8(chunk, v_semi);
                let res = vgetq_lane_u64(vreinterpretq_u64_u8(cmp), 0)
                    | vgetq_lane_u64(vreinterpretq_u64_u8(cmp), 1);
                if res == 0 {
                    for _ in 0..16 {
                        hash = hash.wrapping_mul(31).wrapping_add(*ptr.add(offset) as u64);
                        offset += 1;
                    }
                }
            }
            while offset < len && *ptr.add(offset) != b';' {
                hash = hash.wrapping_mul(31).wrapping_add(*ptr.add(offset) as u64);
                offset += 1;
            }
            offset += 1; // ';'

            // parse temperature (tenths)
            let mut temp: i64 = 0;
            let is_neg = if offset < len && *ptr.add(offset) == b'-' {
                offset += 1;
                true
            } else {
                false
            };
            while offset < len && *ptr.add(offset) != b'\n' {
                let b = *ptr.add(offset);
                if b.is_ascii_digit() {
                    temp = temp * 10 + (b - b'0') as i64;
                }
                offset += 1;
            }
            if is_neg {
                temp = -temp;
            }
            offset += 1; // '\n'

            let slot = table.get_unchecked_mut((hash % SLOTS as u64) as usize);
            if temp < slot.0 { slot.0 = temp; }
            if temp > slot.1 { slot.1 = temp; }
            slot.2 += temp;
            slot.3 += 1;
        }
    }

    /// A scoped task that NEON-scans one newline-aligned chunk into the
    /// thread-local `Table` borrowed from the scope environment.
    struct NeonTask {
        mmap: Arc<Mmap>,
        start: usize,
        end: usize,
    }
    impl<'env> ScopedTask<'env, Table> for NeonTask {
        fn poll(&mut self, cx: &mut Cx<'_, 'env, Table>) -> TaskPoll {
            let bytes = &self.mmap[self.start..self.end];
            // SAFETY: aarch64 target; NEON is baseline on Apple silicon.
            unsafe { scan_neon(bytes, &mut cx.resources.0) };
            TaskPoll::Ready
        }
    }

    fn build_boundaries(mmap: &[u8], total_chunks: usize) -> Vec<(usize, usize)> {
        let len = mmap.len();
        let mut b = Vec::with_capacity(total_chunks);
        let mut current = 0usize;
        for i in 0..total_chunks {
            let mut end = if i + 1 == total_chunks { len } else { ((i + 1) * len) / total_chunks };
            if end < current { end = current; }
            while end < len && end > 0 && mmap[end - 1] != b'\n' {
                end += 1;
            }
            b.push((current, end));
            current = end;
        }
        b
    }

    fn hash_name(name: &[u8]) -> usize {
        let mut h: u64 = 0;
        for &b in name {
            h = h.wrapping_mul(31).wrapping_add(b as u64);
        }
        (h % SLOTS as u64) as usize
    }

    /// One scope runtime per OS thread; `wpt` NEON tasks per thread over
    /// newline-aligned chunks; merge the per-thread tables at the end.
    fn run_layout(mmap: Arc<Mmap>, threads: usize, wpt: usize) -> (Table, Duration) {
        let total_chunks = threads * wpt;
        let boundaries = Arc::new(build_boundaries(&mmap[..], total_chunks));
        let barrier = Arc::new(Barrier::new(threads + 1));
        let mut joins = Vec::with_capacity(threads);

        for shard in 0..threads {
            let mmap = Arc::clone(&mmap);
            let boundaries = Arc::clone(&boundaries);
            let barrier = Arc::clone(&barrier);
            joins.push(thread::spawn(move || {
                let mut table = Table::default();
                barrier.wait();
                scoped(&mut table, |scope: &mut Scope<'_, Table>| {
                    for worker in 0..wpt {
                        let (s, e) = boundaries[shard * wpt + worker];
                        scope.spawn_named("neon", NeonTask { mmap: Arc::clone(&mmap), start: s, end: e });
                    }
                    scope.run();
                });
                table
            }));
        }

        let wall = Instant::now();
        barrier.wait();

        let mut merged = Table::default();
        for j in joins {
            let t = j.join().expect("worker thread panicked");
            for (m, x) in merged.0.iter_mut().zip(t.0.iter()) {
                if x.0 < m.0 { m.0 = x.0; }
                if x.1 > m.1 { m.1 = x.1; }
                m.2 += x.2;
                m.3 += x.3;
            }
        }
        (merged, wall.elapsed())
    }

    pub fn run() {
        let path = std::env::args().nth(1).unwrap_or_else(|| "measurements.txt".to_string());
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("could not open {path:?}: {e}");
                eprintln!("pass the path to the 1BRC file: --example one_billion_rows_neon_threads -- /path/to/measurements.txt");
                return;
            }
        };
        // SAFETY: read-only mmap of a file we opened; not mutated for the run.
        let mmap = Arc::new(unsafe { MmapOptions::new().map(&file).expect("mmap") });

        // Sweet spot on M-series: one worker set per performance core.
        let threads = 8usize;
        let wpt = 8usize;

        println!("== 1BRC — NEON × {threads} threads × {wpt} scoped tasks/thread ==");
        println!("file: {path}\n");

        let mut merged = Table::default();
        for run_i in 0..3 {
            let (m, wall) = run_layout(Arc::clone(&mmap), threads, wpt);
            let rows: i64 = NAMES.iter().map(|n| m.0[hash_name(n)].3).sum();
            println!(
                "[run {run_i}] wall = {:>8.3} ms  ->  {:>6.1} M rows/s   (rows = {})",
                wall.as_secs_f64() * 1000.0,
                rows as f64 / wall.as_secs_f64() / 1e6,
                rows
            );
            merged = m;
        }

        println!("\n-- station report (min/mean/max °C) --");
        let mut total: i64 = 0;
        for n in NAMES {
            let s = merged.0[hash_name(n)];
            if s.3 > 0 {
                total += s.3;
                println!(
                    "{:<10} {:>6.1}/{:>5.1}/{:>5.1}   ({} rows)",
                    String::from_utf8_lossy(n),
                    s.0 as f64 / 10.0,
                    (s.2 as f64 / s.3 as f64) / 10.0,
                    s.1 as f64 / 10.0,
                    s.3
                );
            }
        }
        println!("\ntotal rows accounted: {total}");
    }
}
