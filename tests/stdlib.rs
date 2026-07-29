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

#[test]
fn math_string_list_convert_and_env_extensions_typecheck() {
    let src = r#"
        void Main() {
            assertEq(Math.Clamp(150, 0, 100), 100.0);
            assertEq(Math.Sign(-7), -1);
            assertEq(Math.Log2(8.0), 3.0);
            assertEq(Math.Cbrt(27.0), 3.0);
            assertEq(Math.Hypot(3.0, 4.0), 5.0);

            assertEq("rt".PadLeft(4, "0"), "00rt");
            assertEq("ab".Repeat(3), "ababab");
            assertEq("banana".Count("a"), 3);
            assertEq("hello".Insert(2, "X"), "heXllo");
            assertEq("hello".Remove(1, 3), "ho");
            assertEq(String.Join("-", ["a", "b", "c"]), "a-b-c");
            assertEq(String.Format("{0}:{1}", "x", 7), "x:7");

            var nums = [5, 3, 5, 1, 2];
            var distinct = nums.Distinct().Sort();
            assertEq(distinct.Count, 4);
            assertEq(distinct[0], 1);
            assertEq(distinct[3], 5);
            assertEq(nums.Take(2).Count, 2);
            assertEq(nums.Skip(3).Count, 2);
            assertEq(nums.IndexOf(1), 3);
            assertEq([[1, 2], [3], [4, 5]].Flatten().Count, 5);
            assertEq([1, 2, 3, 4, 5].Chunk(2).Count, 3);
            assertEq(List.Range(0, 3).Count, 3);
            assertEq(List.Range(2, 4)[0], 2);
            assertEq(List.Fill(9, 4).Count, 4);

            assertEq(Convert.ToInt("42"), 42);
            assertEq(Convert.ToFloat("2.5"), 2.5);
            assertEq(Convert.ToBool(0), false);
            assertEq(Convert.ToHex(255), "FF");
            assertEq(Convert.FromHex("FF"), 255);
            assertEq(Convert.ToBinary(10), "1010");
            var bytes = Convert.ToBytes("Hi");
            assertEq(bytes.Count, 2);
            assertEq(Convert.FromBytes(bytes), "Hi");
            var b64 = Convert.ToBase64("Hello");
            assertEq(Convert.FromBase64(b64), "Hello");

            Env.Set("RAYTASK_TEST_ENV", "ok");
            assert(Env.Has("RAYTASK_TEST_ENV"));
            assertEq(Env.Get("RAYTASK_TEST_ENV"), "ok");
            assert(Env.Args.Count >= 1);
            assert(Env.OS.Length > 0);
            assert(Env.CurrentDir.Length > 0);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
}

