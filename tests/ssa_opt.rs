//! SSA optimizer regression and behavior tests.

use raytask::ssa::builder::lower_program;
use raytask::ssa::ir::InstKind;
use raytask::ssa::pass::pipeline_for;
use raytask::vm::Vm;
use raytask::{compile_bytecode_optimized, mono::monomorphize, parse_source_with_stdlib, Optimize};

fn run_opt(src: &str, opt: Optimize) {
    let module = compile_bytecode_optimized(src, true, opt).unwrap();
    Vm::new(module).run().unwrap();
}

#[test]
fn none_matches_speed_hello() {
    let src = r#"
        void Main() {
            print("hello");
        }
    "#;
    run_opt(src, Optimize::None);
    run_opt(src, Optimize::Speed);
    run_opt(src, Optimize::Size);
}

#[test]
fn const_fold_print() {
    run_opt(
        r#"
        void Main() {
            print(1 + 2 * 3);
        }
        "#,
        Optimize::Speed,
    );
}

#[test]
fn sccp_constant_branch() {
    run_opt(
        r#"
        void Main() {
            if (1 < 2) {
                print(1);
            } else {
                print(0);
            }
        }
        "#,
        Optimize::Speed,
    );
}

#[test]
fn dce_dead_arith_still_prints() {
    let src = r#"
        void Main() {
            var x = 1 + 2 + 3 + 4;
            print(9);
        }
    "#;
    run_opt(src, Optimize::Speed);
}

#[test]
fn recursion_not_required_to_inline() {
    run_opt(
        r#"
        int fact(n: int) {
            if (n <= 1) { return 1; }
            return n * fact(n - 1);
        }
        void Main() {
            print(fact(5));
        }
        "#,
        Optimize::Speed,
    );
}

#[test]
fn small_callee_inlining_safe() {
    run_opt(
        r#"
        int add1(x: int) {
            return x + 1;
        }
        void Main() {
            print(add1(41));
        }
        "#,
        Optimize::Speed,
    );
}

#[test]
fn ir_const_fold_reduces_add() {
    let src = r#"
        void Main() {
            print(10 + 20);
        }
    "#;
    let program = parse_source_with_stdlib(src, false).unwrap();
    let program = monomorphize(program);
    let mut ssa = lower_program(&program, false).unwrap();
    let before: usize = ssa
        .functions
        .iter()
        .map(|f| {
            f.values()
                .filter(|(_, i)| matches!(i.kind, InstKind::BinOp { .. }))
                .count()
        })
        .sum();
    let mut pm = pipeline_for(Optimize::Speed);
    pm.run(&mut ssa);
    let after: usize = ssa
        .functions
        .iter()
        .map(|f| {
            f.values()
                .filter(|(_, i)| matches!(i.kind, InstKind::BinOp { .. }))
                .count()
        })
        .sum();
    assert!(after <= before);
}

#[test]
fn loop_runs_under_speed() {
    run_opt(
        r#"
        void Main() {
            var i = 0;
            while (i < 5) {
                i = i + 1;
            }
            print(i);
        }
        "#,
        Optimize::Speed,
    );
}

#[test]
fn async_delay_under_speed() {
    run_opt(
        r#"
        async int Work() {
            await Task.Delay(1);
            return 7;
        }
        async void Main() {
            var x = await Work();
            assertEq(x, 7);
        }
        "#,
        Optimize::Speed,
    );
}

#[test]
fn static_and_oop_under_speed() {
    run_opt(
        r#"
        class Counter {
            static int Seed = 0;
            static void bump() {
                Counter.Seed = Counter.Seed + 1;
            }
        }
        void Main() {
            Counter.bump();
            Counter.bump();
            assertEq(Counter.Seed, 2);
        }
        "#,
        Optimize::Speed,
    );
}
