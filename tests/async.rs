//! Async/await event-loop tests.

use raytask::{check_source, run_source};

#[test]
fn await_delay_and_async_fn() {
    let src = r#"
        async int Work() {
            await Task.Delay(5);
            return 99;
        }
        async void Main() {
            var x = await Work();
            assertEq(x, 99);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
fn task_when_all() {
    let src = r#"
        async void Main() {
            var a = Task.Delay(5);
            var b = Task.Delay(8);
            await Task.WhenAll([a, b]);
            assert(true);
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn task_run_sync_fn() {
    let src = r#"
        async void Main() {
            var v = await Task.Run(() => 3 + 4);
            assertEq(v, 7);
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn await_non_task_is_identity() {
    let src = r#"
        void Main() {
            var x = await 5;
            assertEq(x, 5);
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn concurrent_delays_faster_than_sum() {
    let src = r#"
        async void Main() {
            var t0 = GetTime();
            var a = Task.Delay(40);
            var b = Task.Delay(40);
            await Task.WhenAll([a, b]);
            var elapsed = GetTime() - t0;
            // Should be ~40ms, not ~80ms (allow generous slack on CI)
            assert(elapsed < 120);
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn task_group_cancel_wakes_waiters() {
    let src = r#"
        async int Slow() {
            await Task.Delay(200);
            return 1;
        }

        async void Main() {
            var g = TaskGroup.New();
            g.Run(Slow);
            var all = g.WhenAll();
            g.Cancel();
            try {
                await all;
                assert(false);
            } catch (e) {
                assert(true);
            }
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}
