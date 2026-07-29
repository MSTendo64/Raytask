//! Typechecker registrations for bstd.* built-in types.

use crate::sema::{FuncSig, TypeDef, TypeDefKind};
use crate::span::Span;
use crate::types::Ty;
use std::collections::HashMap;

fn empty_class(name: &str) -> TypeDef {
    TypeDef {
        name: name.into(),
        kind: TypeDefKind::Class,
        type_params: vec![],
        fields: HashMap::new(),
        properties: HashMap::new(),
        methods: HashMap::new(),
        constructors: vec![],
        bases: vec![],
        is_abstract: false,
        span: Span::default(),
    }
}

fn method(name: &str, params: Vec<(&str, Ty)>, ret: Ty, is_static: bool) -> FuncSig {
    FuncSig {
        name: name.into(),
        params: params
            .into_iter()
            .map(|(n, t)| (n.into(), t))
            .collect(),
        ret,
        type_params: vec![],
        is_method: !is_static,
        is_static,
        span: Span::default(),
    }
}

fn ctor(ret: Ty) -> FuncSig {
    FuncSig {
        name: "new".into(),
        params: vec![],
        ret,
        type_params: vec![],
        is_method: false,
        is_static: true,
        span: Span::default(),
    }
}

/// Register File, Math, Json, Dictionary, … into the type map.
pub(crate) fn register_into(types: &mut HashMap<String, TypeDef>) {
    // ---- Dictionary / Set / Queue / Stack ----
    let mut dict = empty_class("Dictionary");
    dict.type_params = vec!["K".into(), "V".into()];
    dict.properties.insert("Count".into(), Ty::Int);
    dict.constructors.push(ctor(Ty::Generic {
        name: "Dictionary".into(),
        args: vec![Ty::TypeParam("K".into()), Ty::TypeParam("V".into())],
    }));
    dict.methods.insert(
        "ContainsKey".into(),
        method("ContainsKey", vec![("key", Ty::TypeParam("K".into()))], Ty::Bool, false),
    );
    dict.methods.insert(
        "Remove".into(),
        method("Remove", vec![("key", Ty::TypeParam("K".into()))], Ty::Void, false),
    );
    dict.methods.insert("Clear".into(), method("Clear", vec![], Ty::Void, false));
    types.insert("Dictionary".into(), dict);

    for (name, add, rem) in [
        ("Set", "Add", "Remove"),
        ("Queue", "Enqueue", "Dequeue"),
        ("Stack", "Push", "Pop"),
    ] {
        let mut td = empty_class(name);
        td.type_params = vec!["T".into()];
        td.properties.insert("Count".into(), Ty::Int);
        td.constructors.push(ctor(Ty::Generic {
            name: name.into(),
            args: vec![Ty::TypeParam("T".into())],
        }));
        td.methods.insert(
            add.into(),
            method(add, vec![("item", Ty::TypeParam("T".into()))], Ty::Void, false),
        );
        if name == "Set" {
            td.methods.insert(
                "Contains".into(),
                method(
                    "Contains",
                    vec![("item", Ty::TypeParam("T".into()))],
                    Ty::Bool,
                    false,
                ),
            );
            td.methods.insert(
                rem.into(),
                method(rem, vec![("item", Ty::TypeParam("T".into()))], Ty::Void, false),
            );
        } else {
            td.methods.insert(
                rem.into(),
                method(rem, vec![], Ty::TypeParam("T".into()), false),
            );
            td.methods.insert(
                "Peek".into(),
                method("Peek", vec![], Ty::TypeParam("T".into()), false),
            );
        }
        types.insert(name.into(), td);
    }

    // ---- static modules ----
    let mut file = empty_class("File");
    for (n, params, ret) in [
        ("ReadText", vec![("path", Ty::String)], Ty::String),
        ("WriteText", vec![("path", Ty::String), ("content", Ty::String)], Ty::Void),
        ("AppendText", vec![("path", Ty::String), ("content", Ty::String)], Ty::Void),
        ("Exists", vec![("path", Ty::String)], Ty::Bool),
        ("Delete", vec![("path", Ty::String)], Ty::Void),
        ("GetInfo", vec![("path", Ty::String)], Ty::Named("FileInfo".into())),
    ] {
        file.methods.insert(
            n.into(),
            method(n, params, ret, true),
        );
    }
    types.insert("File".into(), file);

    let mut info = empty_class("FileInfo");
    info.properties.insert("Path".into(), Ty::String);
    info.properties.insert("Size".into(), Ty::Long);
    info.properties.insert("Created".into(), Ty::Long);
    info.properties.insert("Modified".into(), Ty::Long);
    types.insert("FileInfo".into(), info);

    let mut dir = empty_class("Directory");
    for (n, ret) in [
        ("GetFiles", Ty::Array {
            elem: Box::new(Ty::String),
            dims: 1,
        }),
        ("GetDirectories", Ty::Array {
            elem: Box::new(Ty::String),
            dims: 1,
        }),
        ("Create", Ty::Void),
        ("Delete", Ty::Void),
        ("Exists", Ty::Bool),
    ] {
        dir.methods.insert(
            n.into(),
            method(n, vec![("path", Ty::String)], ret, true),
        );
    }
    types.insert("Directory".into(), dir);

    let mut math = empty_class("Math");
    math.properties.insert("PI".into(), Ty::Double);
    math.properties.insert("E".into(), Ty::Double);
    math.properties.insert("Tau".into(), Ty::Double);
    for (n, arity) in [
        ("Abs", 1),
        ("Sqrt", 1),
        ("Floor", 1),
        ("Ceil", 1),
        ("Round", 1),
        ("Sin", 1),
        ("Cos", 1),
        ("Tan", 1),
        ("Log", 1),
        ("Exp", 1),
        ("Pow", 2),
        ("Min", 2),
        ("Max", 2),
        ("Clamp", 3),
        ("Log2", 1),
        ("Log10", 1),
        ("Atan2", 2),
        ("Sign", 1),
        ("Truncate", 1),
        ("IsNaN", 1),
        ("IsInfinity", 1),
        ("Lerp", 3),
        ("Asin", 1),
        ("Acos", 1),
        ("Atan", 1),
        ("Sinh", 1),
        ("Cosh", 1),
        ("Tanh", 1),
        ("Cbrt", 1),
        ("Hypot", 2),
    ] {
        let params = match arity {
            1 => vec![("x", Ty::Double)],
            2 => vec![("a", Ty::Double), ("b", Ty::Double)],
            _ => vec![("a", Ty::Double), ("b", Ty::Double), ("c", Ty::Double)],
        };
        let ret = match n {
            "Abs" => Ty::Dyn,
            "Sign" => Ty::Int,
            "Truncate" => Ty::Int,
            "IsNaN" | "IsInfinity" => Ty::Bool,
            _ => Ty::Double,
        };
        math.methods.insert(n.into(), method(n, params, ret, true));
    }
    types.insert("Math".into(), math);

    let mut string_mod = empty_class("String");
    string_mod.methods.insert(
        "Join".into(),
        method(
            "Join",
            vec![
                ("sep", Ty::String),
                (
                    "items",
                    Ty::Array {
                        elem: Box::new(Ty::Dyn),
                        dims: 1,
                    },
                ),
            ],
            Ty::String,
            true,
        ),
    );
    string_mod.methods.insert(
        "Format".into(),
        method(
            "Format",
            vec![
                ("template", Ty::String),
                ("a", Ty::Dyn),
                ("b", Ty::Dyn),
            ],
            Ty::String,
            true,
        ),
    );
    string_mod.methods.insert(
        "IsNullOrEmpty".into(),
        method("IsNullOrEmpty", vec![("value", Ty::String)], Ty::Bool, true),
    );
    string_mod.methods.insert(
        "IsNullOrWhiteSpace".into(),
        method(
            "IsNullOrWhiteSpace",
            vec![("value", Ty::String)],
            Ty::Bool,
            true,
        ),
    );
    types.insert("String".into(), string_mod);

    let mut convert = empty_class("Convert");
    for (n, ret) in [
        ("ToInt", Ty::Int),
        ("ToFloat", Ty::Double),
        ("ToBool", Ty::Bool),
        ("ToString", Ty::String),
        ("ToHex", Ty::String),
        ("FromHex", Ty::Int),
        (
            "ToBytes",
            Ty::Array {
                elem: Box::new(Ty::Int),
                dims: 1,
            },
        ),
        ("FromBytes", Ty::String),
        ("ToBase64", Ty::String),
        ("FromBase64", Ty::String),
        ("ToBinary", Ty::String),
    ] {
        convert
            .methods
            .insert(n.into(), method(n, vec![("value", Ty::Dyn)], ret, true));
    }
    types.insert("Convert".into(), convert);

    let mut env = empty_class("Env");
    env.properties.insert("OS".into(), Ty::String);
    env.properties.insert("CurrentDir".into(), Ty::String);
    env.properties.insert("Home".into(), Ty::String);
    env.properties.insert(
        "Args".into(),
        Ty::Array {
            elem: Box::new(Ty::String),
            dims: 1,
        },
    );
    env.methods.insert(
        "Get".into(),
        method("Get", vec![("name", Ty::String)], Ty::String, true),
    );
    env.methods.insert(
        "Set".into(),
        method(
            "Set",
            vec![("name", Ty::String), ("value", Ty::String)],
            Ty::Void,
            true,
        ),
    );
    env.methods.insert(
        "Has".into(),
        method("Has", vec![("name", Ty::String)], Ty::Bool, true),
    );
    types.insert("Env".into(), env);

    let mut random = empty_class("Random");
    random.methods.insert("Next".into(), method("Next", vec![], Ty::Int, true));
    random
        .methods
        .insert("NextDouble".into(), method("NextDouble", vec![], Ty::Double, true));
    types.insert("Random".into(), random);

    let mut dt = empty_class("DateTime");
    dt.properties.insert("Ticks".into(), Ty::Long);
    dt.properties.insert("Utc".into(), Ty::Bool);
    dt.properties.insert("Now".into(), Ty::Named("DateTime".into()));
    dt.properties.insert("UtcNow".into(), Ty::Named("DateTime".into()));
    dt.methods.insert(
        "ToString".into(),
        method("ToString", vec![], Ty::String, false),
    );
    types.insert("DateTime".into(), dt);

    let mut json = empty_class("Json");
    json.methods.insert(
        "Parse".into(),
        method("Parse", vec![("json", Ty::String)], Ty::Dyn, true),
    );
    json.methods.insert(
        "Stringify".into(),
        method(
            "Stringify",
            vec![("value", Ty::Dyn), ("pretty", Ty::Bool)],
            Ty::String,
            true,
        ),
    );
    json.methods.insert(
        "Serialize".into(),
        method("Serialize", vec![("value", Ty::Dyn)], Ty::String, true),
    );
    json.methods.insert(
        "Deserialize".into(),
        method("Deserialize", vec![("json", Ty::String)], Ty::Dyn, true),
    );
    types.insert("Json".into(), json);

    let mut yaml = empty_class("Yaml");
    yaml.methods.insert(
        "Parse".into(),
        method("Parse", vec![("yaml", Ty::String)], Ty::Dyn, true),
    );
    yaml.methods.insert(
        "Serialize".into(),
        method("Serialize", vec![("value", Ty::Dyn)], Ty::String, true),
    );
    yaml.methods.insert(
        "Deserialize".into(),
        method("Deserialize", vec![("yaml", Ty::String)], Ty::Dyn, true),
    );
    types.insert("Yaml".into(), yaml);

    let mut hash = empty_class("Hash");
    for n in ["Sha256", "Sha1", "Md5", "SHA256", "SHA1", "MD5"] {
        hash.methods.insert(
            n.into(),
            method(n, vec![("data", Ty::Dyn)], Ty::String, true),
        );
    }
    types.insert("Hash".into(), hash);

    let mut http = empty_class("Http");
    http.methods.insert(
        "Get".into(),
        method("Get", vec![("url", Ty::String)], Ty::Named("HttpResponse".into()), true),
    );
    http.methods.insert(
        "Post".into(),
        method(
            "Post",
            vec![("url", Ty::String), ("body", Ty::String)],
            Ty::Named("HttpResponse".into()),
            true,
        ),
    );
    http.methods.insert(
        "GetAsync".into(),
        method(
            "GetAsync",
            vec![("url", Ty::String)],
            Ty::Named("HttpResponse".into()),
            true,
        ),
    );
    types.insert("Http".into(), http);

    let mut resp = empty_class("HttpResponse");
    resp.properties.insert("Status".into(), Ty::Int);
    resp.properties.insert("Body".into(), Ty::String);
    types.insert("HttpResponse".into(), resp);

    let mut http_server = empty_class("HttpServer");
    http_server.methods.insert(
        "ServeScript".into(),
        method(
            "ServeScript",
            vec![
                ("host", Ty::String),
                ("port", Ty::Int),
                ("script", Ty::String),
                ("staticDir", Ty::String),
            ],
            Ty::Void,
            true,
        ),
    );
    types.insert("HttpServer".into(), http_server);

    let mut web = empty_class("Web");
    for (name, params, ret) in [
        ("Method", vec![], Ty::String),
        ("Path", vec![], Ty::String),
        ("Body", vec![], Ty::String),
        ("IsHtmx", vec![], Ty::Bool),
        ("ScriptDir", vec![], Ty::String),
        ("StaticDir", vec![], Ty::String),
        ("Render", vec![("path", Ty::String), ("model", Ty::Dyn)], Ty::Void),
        ("Json", vec![("value", Ty::Dyn)], Ty::Void),
        ("Html", vec![("value", Ty::String)], Ty::Void),
        ("Text", vec![("value", Ty::String)], Ty::Void),
        ("File", vec![("path", Ty::String), ("contentType", Ty::String)], Ty::Void),
        ("Write", vec![("value", Ty::String)], Ty::Void),
        ("Redirect", vec![("url", Ty::String)], Ty::Void),
        ("SetStatus", vec![("status", Ty::Int)], Ty::Void),
        ("SetHeader", vec![("name", Ty::String), ("value", Ty::String)], Ty::Void),
        (
            "SetCookie",
            vec![
                ("name", Ty::String),
                ("value", Ty::String),
                ("path", Ty::String),
                ("httpOnly", Ty::Bool),
                ("maxAge", Ty::Int),
            ],
            Ty::Void,
        ),
        ("Query", vec![("name", Ty::String)], Ty::String),
        ("Form", vec![("name", Ty::String)], Ty::String),
        ("Header", vec![("name", Ty::String)], Ty::String),
        ("Cookie", vec![("name", Ty::String)], Ty::String),
        ("ParseJson", vec![("text", Ty::String)], Ty::Dyn),
    ] {
        web.methods
            .insert(name.into(), method(name, params, ret, true));
    }
    types.insert("Web".into(), web);

    let mut template = empty_class("Template");
    template.methods.insert(
        "Render".into(),
        method(
            "Render",
            vec![("path", Ty::String), ("model", Ty::Dyn)],
            Ty::String,
            true,
        ),
    );
    types.insert("Template".into(), template);

    let mut sqlite = empty_class("Sqlite");
    sqlite.methods.insert(
        "Open".into(),
        method(
            "Open",
            vec![("path", Ty::String)],
            Ty::Named("SqliteConnection".into()),
            true,
        ),
    );
    types.insert("Sqlite".into(), sqlite);

    let mut sqlite_conn = empty_class("SqliteConnection");
    sqlite_conn.properties.insert("path".into(), Ty::String);
    sqlite_conn.methods.insert(
        "Execute".into(),
        method("Execute", vec![("sql", Ty::String)], Ty::Int, false),
    );
    sqlite_conn.methods.insert(
        "Query".into(),
        method("Query", vec![("sql", Ty::String)], Ty::Dyn, false),
    );
    sqlite_conn.methods.insert(
        "QueryOne".into(),
        method("QueryOne", vec![("sql", Ty::String)], Ty::Dyn, false),
    );
    sqlite_conn.methods.insert(
        "LastInsertRowId".into(),
        method("LastInsertRowId", vec![], Ty::Long, false),
    );
    sqlite_conn.methods.insert(
        "Close".into(),
        method("Close", vec![], Ty::Void, false),
    );
    types.insert("SqliteConnection".into(), sqlite_conn);

    let mut task = empty_class("Task");
    task.methods.insert(
        "Delay".into(),
        method(
            "Delay",
            vec![("ms", Ty::Int)],
            Ty::Generic {
                name: "Task".into(),
                args: vec![Ty::Void],
            },
            true,
        ),
    );
    task.methods.insert(
        "Run".into(),
        method(
            "Run",
            vec![("fn", Ty::Dyn)],
            Ty::Generic {
                name: "Task".into(),
                args: vec![Ty::Dyn],
            },
            true,
        ),
    );
    task.methods.insert(
        "WhenAll".into(),
        method(
            "WhenAll",
            vec![(
                "tasks",
                Ty::Array {
                    elem: Box::new(Ty::Dyn),
                    dims: 1,
                },
            )],
            Ty::Generic {
                name: "Task".into(),
                args: vec![Ty::Array {
                    elem: Box::new(Ty::Dyn),
                    dims: 1,
                }],
            },
            true,
        ),
    );
    types.insert("Task".into(), task);

    let mut gc = empty_class("Gc");
    gc.methods.insert(
        "Collect".into(),
        method("Collect", vec![], Ty::Int, true),
    );
    gc.methods.insert(
        "Stats".into(),
        method("Stats", vec![], Ty::Dyn, true),
    );
    types.insert("Gc".into(), gc);
    let mut gc2 = empty_class("GC");
    gc2.methods.insert(
        "Collect".into(),
        method("Collect", vec![], Ty::Int, true),
    );
    gc2.methods.insert(
        "Stats".into(),
        method("Stats", vec![], Ty::Dyn, true),
    );
    types.insert("GC".into(), gc2);

    let mut re = empty_class("Regex");
    re.methods.insert(
        "FindAll".into(),
        method(
            "FindAll",
            vec![("text", Ty::String)],
            Ty::Array {
                elem: Box::new(Ty::String),
                dims: 1,
            },
            false,
        ),
    );
    re.methods.insert(
        "IsMatch".into(),
        method("IsMatch", vec![("text", Ty::String)], Ty::Bool, false),
    );
    re.methods.insert(
        "Replace".into(),
        method(
            "Replace",
            vec![("text", Ty::String), ("replacement", Ty::String)],
            Ty::String,
            false,
        ),
    );
    types.insert("Regex".into(), re);

    let mut result = empty_class("Result");
    result.type_params = vec!["T".into(), "E".into()];
    result.properties.insert("IsOk".into(), Ty::Bool);
    result
        .properties
        .insert("Value".into(), Ty::TypeParam("T".into()));
    result
        .properties
        .insert("Error".into(), Ty::TypeParam("E".into()));
    types.insert("Result".into(), result);

    let mut sb = empty_class("StringBuilder");
    sb.properties.insert("Length".into(), Ty::Int);
    sb.constructors.push(ctor(Ty::Named("StringBuilder".into())));
    sb.methods.insert(
        "Append".into(),
        method("Append", vec![("s", Ty::String)], Ty::Void, false),
    );
    sb.methods.insert(
        "ToString".into(),
        method("ToString", vec![], Ty::String, false),
    );
    sb.methods.insert("Clear".into(), method("Clear", vec![], Ty::Void, false));
    types.insert("StringBuilder".into(), sb);

    let mut logger = empty_class("Logger");
    logger.constructors.push(ctor(Ty::Named("Logger".into())));
    for n in ["Log", "Info", "Warn", "Error", "Debug", "LogWarning", "LogError"] {
        logger.methods.insert(
            n.into(),
            method(n, vec![("message", Ty::String)], Ty::Void, false),
        );
    }
    for n in ["Info", "Warn", "Error", "Debug"] {
        // static overloads — typechecker keeps one; instance already registered
        let _ = n;
    }
    types.insert("Logger".into(), logger);

    let mut tcp = empty_class("TcpClient");
    tcp.constructors.push(ctor(Ty::Named("TcpClient".into())));
    tcp.methods.insert(
        "Connect".into(),
        method(
            "Connect",
            vec![("host", Ty::String), ("port", Ty::Int)],
            Ty::Void,
            false,
        ),
    );
    tcp.methods.insert(
        "Send".into(),
        method("Send", vec![("data", Ty::String)], Ty::Void, false),
    );
    tcp.methods.insert(
        "Receive".into(),
        method("Receive", vec![], Ty::String, false),
    );
    tcp.methods.insert("Close".into(), method("Close", vec![], Ty::Void, false));
    types.insert("TcpClient".into(), tcp);

    let mut udp = empty_class("UdpSocket");
    udp.constructors.push(FuncSig {
        name: "new".into(),
        params: vec![("port".into(), Ty::Int)],
        ret: Ty::Named("UdpSocket".into()),
        type_params: vec![],
        is_method: false,
        is_static: true,
        span: Span::default(),
    });
    udp.methods.insert(
        "Send".into(),
        method(
            "Send",
            vec![
                ("data", Ty::String),
                ("host", Ty::String),
                ("port", Ty::Int),
            ],
            Ty::Void,
            false,
        ),
    );
    udp.methods.insert(
        "Receive".into(),
        method("Receive", vec![], Ty::Dyn, false),
    );
    types.insert("UdpSocket".into(), udp);

    // Extra List methods
    if let Some(list) = types.get_mut("List") {
        for (n, ret) in [
            ("Clear", Ty::Void),
            ("First", Ty::TypeParam("T".into())),
            ("Last", Ty::TypeParam("T".into())),
            ("Sum", Ty::Dyn),
            ("Average", Ty::Double),
            ("Max", Ty::TypeParam("T".into())),
            ("Min", Ty::TypeParam("T".into())),
        ] {
            list.methods
                .insert(n.into(), method(n, vec![], ret, false));
        }
        list.methods.insert(
            "Any".into(),
            method("Any", vec![("pred", Ty::Dyn)], Ty::Bool, false),
        );
        list.methods.insert(
            "All".into(),
            method("All", vec![("pred", Ty::Dyn)], Ty::Bool, false),
        );
        list.methods.insert(
            "Where".into(),
            method(
                "Where",
                vec![("pred", Ty::Dyn)],
                Ty::Generic {
                    name: "List".into(),
                    args: vec![Ty::TypeParam("T".into())],
                },
                false,
            ),
        );
        list.methods.insert(
            "Select".into(),
            method(
                "Select",
                vec![("mapper", Ty::Dyn)],
                Ty::Generic {
                    name: "List".into(),
                    args: vec![Ty::Dyn],
                },
                false,
            ),
        );
        list.methods.insert(
            "ParallelMap".into(),
            method(
                "ParallelMap",
                vec![("mapper", Ty::Dyn)],
                Ty::Generic {
                    name: "List".into(),
                    args: vec![Ty::Dyn],
                },
                false,
            ),
        );
    }

    // string.Join static + Split/IndexOf
    if let Some(s) = types.get_mut("string") {
        s.methods.insert(
            "IndexOf".into(),
            method("IndexOf", vec![("s", Ty::String)], Ty::Int, false),
        );
        s.methods.insert(
            "Split".into(),
            method(
                "Split",
                vec![("sep", Ty::String)],
                Ty::Array {
                    elem: Box::new(Ty::String),
                    dims: 1,
                },
                false,
            ),
        );
        s.methods.insert(
            "Join".into(),
            method(
                "Join",
                vec![
                    ("sep", Ty::String),
                    (
                        "parts",
                        Ty::Array {
                            elem: Box::new(Ty::String),
                            dims: 1,
                        },
                    ),
                ],
                Ty::String,
                true,
            ),
        );
    }
}
