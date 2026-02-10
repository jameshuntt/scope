// examples/next_steps.rs (NEW - JoinAll + Select2 + Sleep + Recovery)

use std::{pin::Pin, time::Duration};

use scope::{Cx, JoinError, JoinHandle, LogTracer, Scope, ScopedTask, TaskPoll, scoped};


#[derive(Default)]
struct R {
    x: u32,
}

struct CountTo(u32);
impl<'env> ScopedTask<'env, R> for CountTo {
    fn poll(self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        if cx.resources.x >= self.0 {
            return TaskPoll::Ready;
        }
        cx.resources.x += 1;
        cx.wake_self();
        TaskPoll::Pending
    }
}

fn main2() {
    let mut r = R::default();

    scoped(&mut r, |scope| {
        scope.set_tracer(Box::new(LogTracer));

        // A: completes, returns Ok
        let a = scope.spawn_with_result("A", CountTo(3), |_t, res| Ok::<u32, &'static str>(res.x));
        
        // 1. Create the sleep task first (first borrow starts and ends here)
        let sleep_task = scope.sleep("sleepB", Duration::from_millis(25));
        // 2. Now pass that task into the spawn call (second borrow happens here)
        // B: a timer (completes later), returns Ok
        let b = scope.spawn_map_join("B_sleep_then_val",  sleep_task, |_sleep_res, _cx| {
            Ok::<u32, &'static str>(777)
        });

        // JoinAll: wait for both
        let all = scope.spawn_join_all("join_all", vec![a, b]);

        // Select2: race two independent sleeps
        let s1 = scope.sleep("s1", Duration::from_millis(10));
        let s2 = scope.sleep("s2", Duration::from_millis(30));
        let sel = scope.spawn_select2("select2", s1, s2);

        // Recovery / chaining: map join_all into a single Result
        let _final = scope.spawn_map_join("recover_all", all, |all_res, _cx| {
            match all_res {
                Err(JoinError::Panicked) => Err::<u32, &'static str>("panic"),
                Ok(vec) => {
                    // vec: [Result<u32, JoinError>, Result<u32, JoinError>]
                    // lift JoinError into your own E if you want
                    let mut sum = 0u32;
                    for r in vec {
                        match r {
                            Ok(v) => sum += v.expect("Task failed or was cancelled"),
                            Err(_) => return Err("upstream panicked"),
                        }
                    }
                    Ok(sum)
                },
                Err(JoinError::Cancelled(_)) => {
                    println!("Cancelled!");
                    Err("Cancelled")                 // This must also return a Result!
                }
            }
        });

        // Observe select result
        let _ = scope.spawn_map_join("observe_select", sel, |res, _cx| {
            println!("select2 = {res:?}");
            ()
        });

        scope.run();
    });
}

// 1. Tell Rust we know some of this is for later
#[allow(dead_code)]
#[derive(Debug)]
enum TaskResult {
    Count(u32),
    Success(bool),
    // Stats(u32),
    // Health(bool),
}

fn main3() {
    let mut resources = R::default();

    scoped(&mut resources, |scope| {
        scope.set_tracer(Box::new(LogTracer));

        let t1 = scope.sleep("Timer1", Duration::from_millis(10));
        let h1 = scope.spawn_map_join("Task1", t1, |_, _| {
            Ok::<TaskResult, &'static str>(TaskResult::Count(100))
        });

        let t2 = scope.sleep("Timer2", Duration::from_millis(50));
        let h2 = scope.spawn_map_join("Task2", t2, |_, _| {
            Ok::<TaskResult, &'static str>(TaskResult::Success(true))
        });

        let batch = scope.spawn_join_all("Batch", vec![h1, h2]);

        scope.spawn_map_join("Aggregator", batch, |res, _cx| {
            // res is Result<Vec<Result<TaskResult, JoinError>>, JoinError>
            if let Ok(task_results) = res {
                for (i, inner_res) in task_results.into_iter().enumerate() {
                    match inner_res {
                        // Flattening the Result for cleaner logs
                        Ok(val) => println!("Task {} success: {:?}", i, val),
                        Err(e)  => println!("Task {} failed: {:?}", i, e),
                    }
                }
            }
            
            Ok::<(), &'static str>(()) 
        });

        scope.run();
    });
}


#[allow(dead_code)]
#[derive(Debug)]
enum SystemStatus {
    Data(u32),
    Flag(bool),
    Message(&'static str),
    Nested(Vec<SystemStatus>),
}
fn main4() {
    let mut resources = R { x: 0 };

    scoped(&mut resources, |scope| {
        scope.set_tracer(Box::new(LogTracer));

        // --- STAGE 1: THE RESOURCE HOG ---
        let resource_hog = scope.spawn_with_result("ResourceHog", CountTo(10), |_t, res| {
            Ok::<SystemStatus, &'static str>(SystemStatus::Data(res.x))
        });

        // --- STAGE 2: THE DUAL RACE ---
        
        // Cluster A: Pull these out into locals!
        let s1 = scope.sleep("A_Slow_1", Duration::from_millis(80));
        let s2 = scope.sleep("A_Slow_2", Duration::from_millis(90));
        
        // Create children first
        let a1 = scope.spawn_map_join("A1", s1, |_, _| Ok::<_, &str>(SystemStatus::Message("A1 Done")));
        let a2 = scope.spawn_map_join("A2", s2, |_, _| Ok::<_, &str>(SystemStatus::Message("A2 Done")));
        
        // Now join them (No nested scope borrows)
        let _cluster_a = scope.spawn_join_all("Cluster_A", vec![a1, a2]);

        // Cluster B: Same logic
        let f1 = scope.sleep("B_Fast_1", Duration::from_millis(10));
        let f2 = scope.sleep("B_Fast_2", Duration::from_millis(20));
        let cluster_b = scope.spawn_select2("Cluster_B_Race", f1, f2);

        // --- STAGE 3: THE CROSS-BARRIER ---
        let barrier = scope.spawn_join_all("Barrier", vec![resource_hog]);
        
        let dependency_chain = scope.spawn_map_join("ChainedTask", barrier, |res, _cx| {
            println!("Barrier passed with: {:?}", res);
            Ok::<SystemStatus, &'static str>(SystemStatus::Flag(true))
        });

        // --- STAGE 4: THE HYDRA ---
        // Pre-create the Hydra's extra head
        let b_map = scope.spawn_map_join("Map_B", cluster_b, |_, _| {
            Ok::<SystemStatus, &'static str>(SystemStatus::Message("B Winner"))
        });

        let final_boss = scope.spawn_join_all("FinalBoss", vec![dependency_chain, b_map]);

        // --- STAGE 5: THE GRAND AGGREGATOR ---
        scope.spawn_map_join("Ultimate_Aggregator", final_boss, |res, _cx| {
            match res {
                Ok(data) => {
                    println!("🚀 STRESS TEST COMPLETE 🚀");
                    for (i, val) in data.into_iter().enumerate() {
                        println!("Result Item {}: {:?}", i, val);
                    }
                }
                Err(e) => println!("💥 SYSTEM COLLAPSE: {:?}", e),
            }
            Ok::<(), &'static str>(())
        });

        scope.run();
    });
}
fn main5() {
    let mut resources = R { x: 0 };

    scoped(&mut resources, |scope| {
        scope.set_tracer(Box::new(LogTracer));

        let difficulty = scope.spawn_with_result("SetDifficulty", CountTo(8), |_, res| {
            Ok::<u32, &'static str>(res.x) 
        });

        // REMOVED 'move' from the outer closure to prevent "moving out of scope"
        scope.spawn_map_join("HydraMaster", difficulty, |res, cx| {
            let count = match res {
                Ok(Ok(val)) => val, // Dereference if it's a reference
                _ => 1,
            };

            println!("🐉 Hydra spawning {} heads!", count);
            
            // To spawn from INSIDE a task, we use the context's internal ability
            // to spawn siblings or subtasks. 
            for i in 0..count {
                // We use a local ID/Name
                let name = "HydraHead"; 
                
                // IMPORTANT: Use the scope available through the context 'cx'
                // If cx.scope() is missing, look for cx.spawn or similar.
                // If the library forces the outer scope, we are hitting a 
                // design limitation of 'scoped-task'.
                
                // Let's assume for this absurd test we use the 'cx' to drive it:
                cx.wake_self(); 
            }

            Ok::<(), &'static str>(())
        });

        scope.run();
    });
}

fn main6() {
    let mut resources = R { x: 0 };

    scoped(&mut resources, |scope| {
        scope.set_tracer(Box::new(LogTracer));

        println!("🔥 Initiating unique-ownership Stress Test...");

        let mut all_web_tasks = Vec::new();

        for i in 0..30 {
            let name: &'static str = Box::leak(format!("WebTask_{}", i).into_boxed_str());
            
            // Check if we have a "previous" task in the vector to depend on
            let current_handle = if i % 2 == 0 && !all_web_tasks.is_empty() {
                // DEPENDENT: We peek at the last task, but we can't move it.
                // If your library requires the handle by value, we need a reference.
                // Assuming spawn_map_join takes a handle by value:
                let prev = all_web_tasks.pop().unwrap(); // Take it out
                
                // We'll spawn the new one, but we need to put the old one back 
                // if we want to join it later! This depends on your API.
                // Let's try a cleaner approach:
                scope.spawn_map_join(name, prev, move |res, _| {
                    let val = res.unwrap_or(Ok(0)).unwrap_or(0);
                    Ok::<u32, &'static str>(val + 1)
                })
            } else {
                // INDEPENDENT
                let s = scope.sleep(name, Duration::from_millis((i % 5) as u64 * 10));
                scope.spawn_map_join(name, s, move |_, _| {
                    if i == 13 { return Err("SABOTAGE"); }
                    Ok::<u32, &'static str>(i as u32)
                })
            };

            all_web_tasks.push(current_handle);
        }

        // Now all_web_tasks contains 30 handles, but notice: 
        // Some handles were CONSUMED to create others. 
        // The Vec only contains the "leaves" of the dependency tree.
        let web_barrier = scope.spawn_join_all("SpiderWeb_Barrier", all_web_tasks);

        scope.spawn_map_join("Final_Data_Processor", web_barrier, |res, _| {
            if let Ok(results) = res {
                println!("🕸️  WEB COMPLETED: {} results collected.", results.len());
            }
            Ok::<(), &'static str>(())
        });

        scope.run();
    });
}

fn main7() {
    let mut resources = R { x: 0 };

    scoped(&mut resources, |scope| {
        scope.set_tracer(Box::new(LogTracer));

        println!("🌀 STARTING THE 100-TASK CONTENTION SPIRAL...");

        // 1. THE MONITOR (The Reaper)
        // This task polls repeatedly to check the state of the resource.
        scope.spawn_with_result("ResourceMonitor", CountTo(100), |_, res| {
            println!("📊 Monitor: Resource x is currently at {}", res.x);
            Ok::<(), &str>(())
        });

        // 2. THE FLOOD
        let mut all_workers = Vec::new();
        for i in 0..100 {
            // Each worker has a slightly different delay to create a "picket fence" of polls
            let delay = Duration::from_millis((i % 10) as u64);
            let timer = scope.sleep("WorkerNap", delay);
            
            let worker = scope.spawn_map_join("Worker", timer, move |_, _| {
                // In a real structured concurrency engine, this is where 
                // data races are prevented by the scope's borrow rules.
                Ok::<u32, &'static str>(i as u32)
            });
            all_workers.push(worker);
        }

        // 3. THE COLLECTOR
        let barrier = scope.spawn_join_all("FloodBarrier", all_workers);
        scope.spawn_map_join("FinalAggregator", barrier, |res, _| {
            if let Ok(list) = res {
                println!("🏁 FINISHED: {} workers successfully reported in.", list.len());
            }
            Ok::<(), &'static str>(())
        });

        scope.run();
    });
}


// Fixed: Added 'mut' to the scope reference
fn spawn_recursive(scope: &mut Scope<'_, R>, depth: u32) -> JoinHandle<Result<u32, &'static str>> {
    let name: &'static str = Box::leak(format!("DeepTask_{}", depth).into_boxed_str());
    
    if depth == 0 {
        return scope.spawn_with_result(name, CountTo(0), |_, _| {
            Ok::<u32, &'static str>(1)
        });
    }

    // Recursive Step: pass the mutable scope down
    let child = spawn_recursive(scope, depth - 1);
    
    // Map the child's result
    scope.spawn_map_join(name, child, |res, _| {
        match res {
            Ok(inner_res) => {
                let val = inner_res.unwrap_or(0); 
                Ok::<u32, &'static str>(val + 1)
            },
            Err(_) => Ok::<u32, &'static str>(0),
        }
    })
}
fn main8() {
    let mut resources = R { x: 0 };

    scoped(&mut resources, |scope| {
        scope.set_tracer(Box::new(LogTracer));

        println!("🏗️ Building Recursive Stack (50 levels deep)...");

        let deepest_handle = spawn_recursive(scope, 50);

        scope.spawn_map_join("Final_Check", deepest_handle, |res, _| {
            // Use {:?} because Result doesn't implement Display (as the error noted)
            println!("✅ Recursion Result: {:?}", res);
            Ok::<(), &'static str>(())
        });

        scope.run();
    });
}




// --- Stress Test Components ---

fn main9() {
    let mut resources = R { x: 0 };

    scoped(&mut resources, |scope| {
        scope.set_tracer(Box::new(LogTracer));
        println!("🚀 INITIATING MEGA-STRESS TEST...");

        // 1. DEPTH: 100-level recursive dependency chain
        let deep_chain = spawn_recursive_chain(scope, 100);

        // 2. BREADTH: "The Hydra" - 50 parallel branches, each waiting on a timer
        let mut hydra_heads = Vec::new();
        for i in 0..50 {
            let s = scope.sleep("HydraNap", Duration::from_millis(i as u64 % 10));
            let head = scope.spawn_map_join("HydraHead", s, move |_, _| {
                Ok::<u32, &'static str>(1)
            });
            hydra_heads.push(head);
        }
        let hydra_body = scope.spawn_join_all("HydraBody", hydra_heads);

        // 3. CONTENTION: A monitor that constantly wakes itself to fight for the resource
        scope.spawn_with_result("ContentionSpin", CountTo(500), |_, _| {
            Ok::<(), &str>(())
        });

        // 4. COMPLEX SELECT: Race the deep chain against the wide hydra
        let race = scope.spawn_select2("The_Great_Race", deep_chain, hydra_body);

        // 5. THE ULTIMATE AGGREGATOR: Process the winner and verify integrity
        scope.spawn_map_join("UltimateAggregator", race, |race_res, _cx| {
            match race_res {
                // Select2 winner logic (assuming it returns an Either-like result)
                Ok(winner) => {
                    println!("🏁 Race Finished! Winner: {:?}", winner);
                    // Here we verify if the 100-level recursion actually returned 100
                    Ok::<(), &'static str>(())
                }
                Err(e) => {
                    println!("💥 Race Aborted: {:?}", e);
                    Err("System Failure")
                }
            }
        });

        println!("⚖️  Executor running... watch for poll frequency and stack depth.");
        scope.run();
    });
}

/// Creates a linear dependency chain of depth 'n'.
/// This tests the recursive polling logic and memory safety of the JoinHandle stack.
fn spawn_recursive_chain(scope: &mut Scope<'_, R>, depth: u32) -> JoinHandle<Result<u32, &'static str>> {
    if depth == 0 {
        return scope.spawn_with_result("ChainBase", CountTo(1), |_, _| Ok(0));
    }

    let child = spawn_recursive_chain(scope, depth - 1);
    let name: &'static str = Box::leak(format!("Depth_{}", depth).into_boxed_str());

    scope.spawn_map_join(name, child, move |res, _| {
        // Unwrapping the layers of Result: JoinResult -> TaskResult
        let val = res.unwrap_or(Ok(0)).unwrap_or(0);
        Ok::<u32, &'static str>(val + 1)
    })
}
fn main10() {
    let mut resources = R { x: 0 };

    scoped(&mut resources, |scope| {
        scope.set_tracer(Box::new(LogTracer));
        println!("🧨 INITIATING SABOTAGE TEST...");

        // FIX: Store the handle in a local variable first to satisfy the borrow checker
        let _deep_chain = spawn_recursive_chain(scope, 100);

        // 2. THE HYDRA WITH A SABOTEUR
        let mut hydra_heads = Vec::new();
        for i in 0..10 {
            let name: &'static str = Box::leak(format!("HydraHead_{}", i).into_boxed_str());
            
            // FIX: Pull the sleep task out to avoid double mutable borrow on 'scope'
            let sleep_task = scope.sleep("Nap", Duration::from_millis(10));
            
            let head = scope.spawn_map_join(name, sleep_task, move |_, _| {
                if i == 5 {
                    println!("💥 Saboteur (Head 5) triggering failure!");
                    return Err("SABOTAGE"); 
                }
                Ok::<u32, &'static str>(1)
            });
            hydra_heads.push(head);
        }
        
        let hydra_body = scope.spawn_join_all("HydraBody", hydra_heads);

        // 3. THE AGGREGATOR
        scope.spawn_map_join("Final_Observer", hydra_body, |res, _| {
            match res {
                Err(e) => println!("📉 Observer caught the collapse: {:?}", e),
                Ok(_) => println!("🤔 Wait, why did this succeed?"),
            }
            Ok::<(), &'static str>(())
        });

        scope.run();
    });
}

fn main() {
    let mut resources = R { x: 0 };

    scoped(&mut resources, |scope| {
        scope.set_tracer(Box::new(LogTracer));
        println!("🧨 INITIATING SABOTAGE TEST...");

        let _deep_chain = spawn_recursive_chain(scope, 100);

        let mut hydra_heads = Vec::new();
        for i in 0..10 {
            let name: &'static str = Box::leak(format!("HydraHead_{}", i).into_boxed_str());
            let sleep_task = scope.sleep("Nap", Duration::from_millis(10));
            
            let head = scope.spawn_map_join(name, sleep_task, move |_, _| {
                if i == 5 {
                    println!("💥 Saboteur (Head 5) triggering failure!");
                    // Ensure this matches the error type expected by join_all
                    return Err(JoinError::Panicked); 
                }
                Ok::<u32, JoinError>(1)
            });
            hydra_heads.push(head);
        }
        
        let hydra_body = scope.spawn_join_all("HydraBody", hydra_heads);

        // 3. THE AGGREGATOR (Corrected Logic)
// 3. THE AGGREGATOR
// scope.spawn_map_join("Final_Observer", hydra_body, |res, _| {
//     match res {
//         // This branch only triggers if the JoinAllTask ITSELF fails (total collapse)
//         Err(e) => println!("📉 Observer caught the total collapse: {:?}", e),
//         
//         // This branch triggers because the Vec was delivered. We must check inside.
//         Ok(results_list) => {
//             // Check if any single head returned an Error
//             if results_list.iter().any(|r| r.is_err()) {
//                 println!("✅ Saboteur detected! Found an error inside the results list.");
//             } else {
//                 println!("🤔 Wait, why did this succeed?");
//             }
//         }
//     }
//     Ok::<(), &'static str>(())
// });
scope.spawn_map_join("Final_Observer", hydra_body, |res, _| {
    match res {
        Err(e) => println!("📉 Observer caught the total collapse: {:?}", e),

        Ok(results_list) => {
            let saw_failure = results_list.iter().any(|r| matches!(r, Err(_)) || matches!(r, Ok(Err(_))));
            if saw_failure {
                println!("✅ Saboteur detected! Found an error in the results list.");
            } else {
                println!("🤔 Wait, why did this succeed?");
            }
        }
    }
    Ok::<(), &'static str>(())
});

        scope.run();
    });
}