//! Standard library integration tests.

use raytask::{check_source, run_source};

#[test]
fn collections_and_math() {
    let src = r#"
        void Main() {
            var list = new List<int>();
            list.Add(10);
            list.Add(20);
            assert(list.Count == 2);
            assertEq(list.Sum(), 30);
            assertEq(Math.Sqrt(9.0), 3.0);
            var t = "abc".ToUpper();
            assertEq(t, "ABC");
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
fn json_and_result() {
    let src = r#"
        void Main() {
            var obj = Json.Parse("{\"n\": 7}");
            assertEq(obj["n"], 7);
            var r = Ok("yes");
            assert(r.IsOk);
            assertEq(r.Value, "yes");
            var e = Error("no");
            assert(!e.IsOk);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
fn dict_set_queue_stack() {
    let src = r#"
        void Main() {
            var d = new Dictionary<string, int>();
            d["k"] = 5;
            assert(d.ContainsKey("k"));
            assertEq(d["k"], 5);

            var s = new Set<int>();
            s.Add(1);
            s.Add(1);
            assertEq(s.Count, 1);

            var q = new Queue<string>();
            q.Enqueue("a");
            q.Enqueue("b");
            assertEq(q.Dequeue(), "a");

            var st = new Stack<int>();
            st.Push(1);
            st.Push(2);
            assertEq(st.Pop(), 2);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
fn string_builder_and_join() {
    let src = r#"
        void Main() {
            var sb = new StringBuilder();
            sb.Append("Hello");
            sb.Append(" ");
            sb.Append("World");
            assertEq(sb.ToString(), "Hello World");
            var j = string.Join("-", ["a", "b"]);
            assertEq(j, "a-b");
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
fn fs_roundtrip() {
    let src = r#"
        void Main() {
            File.WriteText("target/_rt_fs_test.txt", "ping");
            assert(File.Exists("target/_rt_fs_test.txt"));
            assertEq(File.ReadText("target/_rt_fs_test.txt"), "ping");
            File.Delete("target/_rt_fs_test.txt");
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
fn crypto_hash() {
    let src = r#"
        void Main() {
            var h = Hash.Md5("abc");
            assert(h.Length == 32);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
fn accepts_stdlib_demo_types() {
    let src = std::fs::read_to_string("examples/stdlib_demo.rt").unwrap();
    let report = check_source(&src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
}
