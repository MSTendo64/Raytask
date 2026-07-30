use raytask::{check_source, run_source};

#[test]
fn static_method_call_through_type_name() {
    let src = r#"
        class MathEx {
            static int Add(a: int, b: int) { return a + b; }
        }

        void Main() {
            assertEq(MathEx.Add(20, 22), 42);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
fn static_fields_and_properties_work_through_type() {
    let src = r#"
        class Counter {
            static int Seed = 5;
            static property Value: int { get; set; }
        }

        void Main() {
            assertEq(Counter.Seed, 5);
            Counter.Value = 42;
            assertEq(Counter.Value, 42);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
fn interface_contract_is_enforced() {
    let src = r#"
        interface Named {
            string GetName();
        }

        class BadNamed : Named {
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(
        !report.ok(),
        "expected interface contract failure, got:\n{}",
        report.format_all()
    );
    assert!(report.format_all().contains("does not implement interface method"));
}

#[test]
fn overriding_requires_override_keyword() {
    let src = r#"
        class Base {
            virtual int Value() { return 1; }
        }

        class Child : Base {
            int Value() { return 2; }
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(!report.ok(), "expected override enforcement failure");
    assert!(report.format_all().contains("mark it as override"));
}

#[test]
fn generic_type_constraints_are_checked() {
    let src = r#"
        interface Named {
            string GetName();
        }

        class Person : Named {
            string GetName() { return "ok"; }
        }

        class Box<T> where T: Named {
        }

        void Main() {
            Box<Person> ok;
            Box<int> bad;
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(
        !report.ok(),
        "expected generic constraint failure, got:\n{}",
        report.format_all()
    );
    assert!(report.format_all().contains("must satisfy"));
}

#[test]
fn task_when_any_returns_first_result() {
    let src = r#"
        async void Main() {
            var x = await Task.WhenAny([
                Task.Run(() => 7),
                Task.Run(() => 9)
            ]);
            assert(x == 7 || x == 9);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
fn cancellation_token_source_changes_token_state() {
    let src = r#"
        void Main() {
            var src = CancellationTokenSource.New();
            assert(!src.Token.IsCancellationRequested);
            src.Cancel();
            assert(src.Token.IsCancellationRequested);
            try {
                src.Token.ThrowIfCancellationRequested();
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

#[test]
fn task_group_aggregates_spawned_tasks() {
    let src = r#"
        async void Main() {
            var group = TaskGroup.New();
            group.Run(() => 10);
            group.Run(() => 32);

            var first = await group.WhenAny();
            assert(first == 10 || first == 32);

            var all = await group.WhenAll();
            assertEq(all.Length, 2);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}
