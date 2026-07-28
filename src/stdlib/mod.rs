//! RayTask standard library natives (bstd.*).

pub mod ids;

mod collections;
mod crypto;
mod fs;
mod io;
mod json;
mod logging;
mod math;
mod net;
mod regex_ops;
mod result_ops;
mod string_ops;
mod test_ops;
mod time;
mod unsafe_mem;
mod yaml;

use crate::error::RuntimeResult;
use crate::value::{ObjectInstance, Value};
use ids::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Resolve a built-in global by name.
pub fn builtin_global(name: &str) -> Option<Value> {
    Some(match name {
        "print" => Value::Native(PRINT),
        "write" => Value::Native(WRITE),
        "readLine" => Value::Native(READ_LINE),
        "readKey" => Value::Native(READ_KEY),
        "sleep" => Value::Native(SLEEP),
        "ParseInt" | "int" => Value::Native(PARSE_INT),
        "ParseFloat" => Value::Native(PARSE_FLOAT),
        "ToString" => Value::Native(TO_STRING),
        "IsNull" => Value::Native(IS_NULL),
        "IsNotNull" => Value::Native(IS_NOT_NULL),
        "IsNumeric" => Value::Native(IS_NUMERIC),
        "IsAlpha" => Value::Native(IS_ALPHA),
        "IsEmail" => Value::Native(IS_EMAIL),
        "RandomInt" => Value::Native(RANDOM_INT),
        "GenerateGuid" => Value::Native(GENERATE_GUID),
        "GetTime" => Value::Native(GET_TIME),
        "gc" => Value::Native(GC_COLLECT),
        "assert" => Value::Native(ASSERT),
        "assertEq" => Value::Native(ASSERT_EQ),
        "malloc" => Value::Native(MALLOC),
        "free" => Value::Native(FREE),
        "sizeof" => Value::Native(SIZEOF),
        "regex" => Value::Native(REGEX_NEW),
        "Ok" => Value::Native(OK),
        "Error" => Value::Native(ERR),
        "File" => Value::TypeModule("File".into()),
        "Directory" => Value::TypeModule("Directory".into()),
        "Math" => Value::TypeModule("Math".into()),
        "Json" => Value::TypeModule("Json".into()),
        "Yaml" => Value::TypeModule("Yaml".into()),
        "Hash" => Value::TypeModule("Hash".into()),
        "Http" => Value::TypeModule("Http".into()),
        "Task" => Value::TypeModule("Task".into()),
        "Thread" => Value::TypeModule("Thread".into()),
        "Gc" | "GC" => Value::TypeModule("Gc".into()),
        "DateTime" => Value::TypeModule("DateTime".into()),
        "Random" => Value::TypeModule("Random".into()),
        "Logger" => Value::TypeModule("Logger".into()),
        "string" => Value::TypeModule("string".into()),
        _ => return None,
    })
}

/// Property / method lookup on values (instance + static modules).
pub fn get_property(obj: &Value, name: &str) -> RuntimeResult<Value> {
    match obj {
        Value::TypeModule(module) => static_member(module, name),
        Value::Object(o) => {
            let o = o.borrow();
            if let Some(v) = o.fields.get(name) {
                return Ok(v.clone());
            }
            match o.class_name.as_str() {
                "Set" => match name {
                    "Count" | "Length" => {
                        let items = o.fields.get("items").cloned().unwrap_or(Value::Null);
                        Ok(Value::Int(array_len(&items)))
                    }
                    "Add" => Ok(Value::Native(SET_ADD)),
                    "Contains" => Ok(Value::Native(SET_CONTAINS)),
                    "Remove" => Ok(Value::Native(SET_REMOVE)),
                    _ => Err(undef(&o.class_name, name)),
                },
                "Queue" => match name {
                    "Count" | "Length" => {
                        let items = o.fields.get("items").cloned().unwrap_or(Value::Null);
                        Ok(Value::Int(array_len(&items)))
                    }
                    "Enqueue" => Ok(Value::Native(QUEUE_ENQUEUE)),
                    "Dequeue" => Ok(Value::Native(QUEUE_DEQUEUE)),
                    "Peek" => Ok(Value::Native(QUEUE_PEEK)),
                    _ => Err(undef(&o.class_name, name)),
                },
                "Stack" => match name {
                    "Count" | "Length" => {
                        let items = o.fields.get("items").cloned().unwrap_or(Value::Null);
                        Ok(Value::Int(array_len(&items)))
                    }
                    "Push" => Ok(Value::Native(STACK_PUSH)),
                    "Pop" => Ok(Value::Native(STACK_POP)),
                    "Peek" => Ok(Value::Native(STACK_PEEK)),
                    _ => Err(undef(&o.class_name, name)),
                },
                "StringBuilder" => match name {
                    "Length" => {
                        let s = o
                            .fields
                            .get("buf")
                            .map(|v| v.as_string())
                            .unwrap_or_default();
                        Ok(Value::Int(s.len() as i64))
                    }
                    "Append" => Ok(Value::Native(SB_APPEND)),
                    "ToString" => Ok(Value::Native(SB_TO_STRING)),
                    "Clear" => Ok(Value::Native(SB_CLEAR)),
                    _ => Err(undef(&o.class_name, name)),
                },
                "Regex" => match name {
                    "FindAll" => Ok(Value::Native(RE_FIND_ALL)),
                    "IsMatch" => Ok(Value::Native(RE_IS_MATCH)),
                    "Replace" => Ok(Value::Native(RE_REPLACE)),
                    _ => Err(undef(&o.class_name, name)),
                },
                "Result" => match name {
                    "IsOk" => Ok(Value::Bool(
                        o.fields
                            .get("ok")
                            .map(|v| v.is_truthy())
                            .unwrap_or(false),
                    )),
                    "Value" => Ok(o.fields.get("value").cloned().unwrap_or(Value::Null)),
                    "Error" => Ok(o.fields.get("error").cloned().unwrap_or(Value::Null)),
                    _ => Err(undef(&o.class_name, name)),
                },
                "FileInfo" => match name {
                    "Size" | "Created" | "Modified" | "Path" => Ok(o
                        .fields
                        .get(name)
                        .cloned()
                        .unwrap_or(Value::Null)),
                    _ => Err(undef(&o.class_name, name)),
                },
                "TcpClient" => match name {
                    "Connect" => Ok(Value::Native(TCP_CONNECT)),
                    "Send" => Ok(Value::Native(TCP_SEND)),
                    "Receive" => Ok(Value::Native(TCP_RECEIVE)),
                    "Close" => Ok(Value::Native(TCP_CLOSE)),
                    _ => Err(undef(&o.class_name, name)),
                },
                "UdpSocket" => match name {
                    "Send" => Ok(Value::Native(UDP_SEND)),
                    "Receive" => Ok(Value::Native(UDP_RECEIVE)),
                    _ => Err(undef(&o.class_name, name)),
                },
                "Logger" => match name {
                    "Info" | "Log" => Ok(Value::Native(LOG_INFO)),
                    "Warn" | "LogWarning" => Ok(Value::Native(LOG_WARN)),
                    "Error" | "LogError" => Ok(Value::Native(LOG_ERROR)),
                    "Debug" => Ok(Value::Native(LOG_DEBUG)),
                    _ => Err(undef(&o.class_name, name)),
                },
                _ => Err(crate::error::RuntimeError::UndefinedVariable(name.into())),
            }
        }
        Value::Array(a) => match name {
            "Length" | "Count" => Ok(Value::Int(a.borrow().len() as i64)),
            "Add" => Ok(Value::Native(LIST_ADD)),
            "Get" => Ok(Value::Native(LIST_GET)),
            "RemoveAt" => Ok(Value::Native(LIST_REMOVE_AT)),
            "Contains" => Ok(Value::Native(LIST_CONTAINS)),
            "Clear" => Ok(Value::Native(LIST_CLEAR)),
            "Sum" => Ok(Value::Native(LIST_SUM)),
            "Average" => Ok(Value::Native(LIST_AVERAGE)),
            "Max" => Ok(Value::Native(LIST_MAX)),
            "Min" => Ok(Value::Native(LIST_MIN)),
            "First" => Ok(Value::Native(LIST_FIRST)),
            "Last" => Ok(Value::Native(LIST_LAST)),
            "Any" => Ok(Value::Native(LIST_ANY)),
            "All" => Ok(Value::Native(LIST_ALL)),
            "Where" => Ok(Value::Native(LIST_WHERE)),
            "Select" => Ok(Value::Native(LIST_SELECT)),
            "ParallelMap" => Ok(Value::Native(LIST_PARALLEL_MAP)),
            _ => Err(crate::error::RuntimeError::UndefinedVariable(format!(
                "List.{}",
                name
            ))),
        },
        Value::Dict(d) => match name {
            "Count" | "Length" => Ok(Value::Int(d.borrow().len() as i64)),
            "ContainsKey" => Ok(Value::Native(DICT_CONTAINS_KEY)),
            "Remove" => Ok(Value::Native(DICT_REMOVE)),
            "Clear" => Ok(Value::Native(DICT_CLEAR)),
            "Keys" => Ok(Value::Native(DICT_KEYS)),
            "Values" => Ok(Value::Native(DICT_VALUES)),
            _ => Err(crate::error::RuntimeError::UndefinedVariable(format!(
                "Dictionary.{}",
                name
            ))),
        },
        Value::String(s) => match name {
            "Length" => Ok(Value::Int(s.chars().count() as i64)),
            "ToUpper" => Ok(Value::Native(STR_TO_UPPER)),
            "ToLower" => Ok(Value::Native(STR_TO_LOWER)),
            "Trim" => Ok(Value::Native(STR_TRIM)),
            "TrimStart" => Ok(Value::Native(STR_TRIM_START)),
            "TrimEnd" => Ok(Value::Native(STR_TRIM_END)),
            "Contains" => Ok(Value::Native(STR_CONTAINS)),
            "StartsWith" => Ok(Value::Native(STR_STARTS_WITH)),
            "EndsWith" => Ok(Value::Native(STR_ENDS_WITH)),
            "IndexOf" => Ok(Value::Native(STR_INDEX_OF)),
            "Replace" => Ok(Value::Native(STR_REPLACE)),
            "Substring" => Ok(Value::Native(STR_SUBSTRING)),
            "Split" => Ok(Value::Native(STR_SPLIT)),
            "ToString" => Ok(Value::String(s.clone())),
            _ => Err(crate::error::RuntimeError::UndefinedVariable(format!(
                "string.{}",
                name
            ))),
        },
        Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::UInt(_) | Value::Char(_)
            if name == "ToString" =>
        {
            Ok(Value::String(obj.as_string().into()))
        }
        _ => Err(crate::error::RuntimeError::TypeError(format!(
            "cannot get property '{}' on {}",
            name,
            obj.type_name()
        ))),
    }
}

fn static_member(module: &str, name: &str) -> RuntimeResult<Value> {
    let id = match (module, name) {
        ("File", "ReadText") => FILE_READ_TEXT,
        ("File", "WriteText") => FILE_WRITE_TEXT,
        ("File", "AppendText") => FILE_APPEND_TEXT,
        ("File", "ReadBytes") => FILE_READ_BYTES,
        ("File", "WriteBytes") => FILE_WRITE_BYTES,
        ("File", "Exists") => FILE_EXISTS,
        ("File", "Delete") => FILE_DELETE,
        ("File", "GetInfo") => FILE_GET_INFO,
        ("Directory", "GetFiles") => DIR_GET_FILES,
        ("Directory", "GetDirectories") => DIR_GET_DIRS,
        ("Directory", "Create") => DIR_CREATE,
        ("Directory", "Delete") => DIR_DELETE,
        ("Directory", "Exists") => DIR_EXISTS,
        ("Math", "Abs") => MATH_ABS,
        ("Math", "Sqrt") => MATH_SQRT,
        ("Math", "Pow") => MATH_POW,
        ("Math", "Floor") => MATH_FLOOR,
        ("Math", "Ceil") => MATH_CEIL,
        ("Math", "Round") => MATH_ROUND,
        ("Math", "Min") => MATH_MIN,
        ("Math", "Max") => MATH_MAX,
        ("Math", "Sin") => MATH_SIN,
        ("Math", "Cos") => MATH_COS,
        ("Math", "Tan") => MATH_TAN,
        ("Math", "Log") => MATH_LOG,
        ("Math", "Exp") => MATH_EXP,
        ("Math", "PI") => return Ok(Value::Float(std::f64::consts::PI)),
        ("Math", "E") => return Ok(Value::Float(std::f64::consts::E)),
        ("DateTime", "Now") => return time::now(false),
        ("DateTime", "UtcNow") => return time::now(true),
        ("Random", "Next") => RANDOM_NEXT,
        ("Random", "NextDouble") => RANDOM_NEXT_DOUBLE,
        ("Json", "Parse") => JSON_PARSE,
        ("Json", "Stringify") => JSON_STRINGIFY,
        ("Json", "Serialize") => JSON_SERIALIZE,
        ("Json", "Deserialize") => JSON_DESERIALIZE,
        ("Yaml", "Parse") => YAML_PARSE,
        ("Yaml", "Serialize") => YAML_SERIALIZE,
        ("Yaml", "Deserialize") => YAML_DESERIALIZE,
        ("Hash", "Sha256") => HASH_SHA256,
        ("Hash", "SHA256") => HASH_SHA256,
        ("Hash", "Sha1") => HASH_SHA1,
        ("Hash", "SHA1") => HASH_SHA1,
        ("Hash", "Md5") | ("Hash", "MD5") => HASH_MD5,
        ("Http", "Get") => HTTP_GET,
        ("Http", "Post") => HTTP_POST,
        ("Http", "GetAsync") => HTTP_GET_ASYNC,
        ("Task", "Delay") => TASK_DELAY,
        ("Task", "Run") => TASK_RUN,
        ("Task", "WhenAll") => TASK_WHEN_ALL,
        ("Thread", "Run") => THREAD_RUN,
        ("Thread", "Sleep") => THREAD_SLEEP,
        ("Gc", "Collect") | ("GC", "Collect") => GC_COLLECT,
        ("Gc", "Stats") | ("GC", "Stats") => GC_STATS,
        ("Logger", "Info") => LOG_INFO,
        ("Logger", "Warn") => LOG_WARN,
        ("Logger", "Error") => LOG_ERROR,
        ("Logger", "Debug") => LOG_DEBUG,
        ("string", "Join") => STR_JOIN,
        ("string", "IsNullOrEmpty") => {
            // return a native that checks string
            return Ok(Value::Native(IS_NULL)); // fallback; real check in call if needed
        }
        _ => {
            return Err(crate::error::RuntimeError::UndefinedVariable(format!(
                "{}.{}",
                module, name
            )))
        }
    };
    Ok(Value::Native(id))
}

fn undef(class: &str, name: &str) -> crate::error::RuntimeError {
    crate::error::RuntimeError::UndefinedVariable(format!("{}.{}", class, name))
}

fn array_len(v: &Value) -> i64 {
    match v {
        Value::Array(a) => a.borrow().len() as i64,
        _ => 0,
    }
}

/// Dispatch a native call. `args` already includes receiver as first arg for methods.
pub fn call_native(id: usize, args: &[Value]) -> RuntimeResult<Value> {
    match id {
        // globals / io
        PRINT => io::print_ln(args),
        WRITE => io::write(args),
        READ_LINE => io::read_line(),
        READ_KEY => io::read_key(),
        SLEEP | THREAD_SLEEP => io::sleep(args),
        PARSE_INT => io::parse_int(args),
        PARSE_FLOAT => io::parse_float(args),
        TO_STRING => Ok(Value::String(
            args.first().map(|v| v.as_string()).unwrap_or_default().into(),
        )),
        IS_NULL => Ok(Value::Bool(matches!(
            args.first().unwrap_or(&Value::Null),
            Value::Null
        ))),
        IS_NOT_NULL => Ok(Value::Bool(!matches!(
            args.first().unwrap_or(&Value::Null),
            Value::Null
        ))),
        IS_NUMERIC => io::is_numeric(args),
        IS_ALPHA => io::is_alpha(args),
        IS_EMAIL => io::is_email(args),
        RANDOM_INT => math::random_int(args),
        GENERATE_GUID => Ok(Value::String(uuid::Uuid::new_v4().to_string().into())),
        GET_TIME => time::get_time_ms(),
        ASSERT => test_ops::assert_true(args),
        ASSERT_EQ => test_ops::assert_eq(args),
        MALLOC => unsafe_mem::malloc(args),
        FREE => unsafe_mem::free(args),
        SIZEOF => unsafe_mem::sizeof_val(args),
        REGEX_NEW => regex_ops::regex_new(args),
        OK => result_ops::ok(args),
        ERR => result_ops::err(args),

        // collections
        LIST_ADD => collections::list_add(args),
        LIST_GET => collections::list_get(args),
        LIST_REMOVE_AT => collections::list_remove_at(args),
        LIST_CONTAINS => collections::list_contains(args),
        LIST_CLEAR => collections::list_clear(args),
        LIST_SUM => collections::list_sum(args),
        LIST_AVERAGE => collections::list_average(args),
        LIST_MAX => collections::list_max(args),
        LIST_MIN => collections::list_min(args),
        LIST_FIRST => collections::list_first(args),
        LIST_LAST => collections::list_last(args),
        LIST_ANY | LIST_ALL | LIST_WHERE | LIST_SELECT => collections::list_linq_stub(id, args),

        DICT_CONTAINS_KEY => collections::dict_contains_key(args),
        DICT_REMOVE => collections::dict_remove(args),
        DICT_CLEAR => collections::dict_clear(args),
        DICT_KEYS => collections::dict_keys(args),
        DICT_VALUES => collections::dict_values(args),

        SET_ADD => collections::set_add(args),
        SET_CONTAINS => collections::set_contains(args),
        SET_REMOVE => collections::set_remove(args),
        QUEUE_ENQUEUE => collections::queue_enqueue(args),
        QUEUE_DEQUEUE => collections::queue_dequeue(args),
        QUEUE_PEEK => collections::queue_peek(args),
        STACK_PUSH => collections::stack_push(args),
        STACK_POP => collections::stack_pop(args),
        STACK_PEEK => collections::stack_peek(args),

        // string
        STR_TO_UPPER => string_ops::to_upper(args),
        STR_TO_LOWER => string_ops::to_lower(args),
        STR_TRIM => string_ops::trim(args),
        STR_TRIM_START => string_ops::trim_start(args),
        STR_TRIM_END => string_ops::trim_end(args),
        STR_CONTAINS => string_ops::contains(args),
        STR_STARTS_WITH => string_ops::starts_with(args),
        STR_ENDS_WITH => string_ops::ends_with(args),
        STR_INDEX_OF => string_ops::index_of(args),
        STR_REPLACE => string_ops::replace(args),
        STR_SUBSTRING => string_ops::substring(args),
        STR_SPLIT => string_ops::split(args),
        STR_JOIN => string_ops::join(args),

        SB_APPEND => string_ops::sb_append(args),
        SB_TO_STRING => string_ops::sb_to_string(args),
        SB_CLEAR => string_ops::sb_clear(args),

        RE_FIND_ALL => regex_ops::find_all(args),
        RE_IS_MATCH => regex_ops::is_match(args),
        RE_REPLACE => regex_ops::replace(args),

        // fs
        FILE_READ_TEXT => fs::read_text(args),
        FILE_WRITE_TEXT => fs::write_text(args),
        FILE_APPEND_TEXT => fs::append_text(args),
        FILE_READ_BYTES => fs::read_bytes(args),
        FILE_WRITE_BYTES => fs::write_bytes(args),
        FILE_EXISTS => fs::exists(args),
        FILE_DELETE => fs::delete_file(args),
        FILE_GET_INFO => fs::get_info(args),
        DIR_GET_FILES => fs::get_files(args),
        DIR_GET_DIRS => fs::get_dirs(args),
        DIR_CREATE => fs::create_dir(args),
        DIR_DELETE => fs::delete_dir(args),
        DIR_EXISTS => fs::dir_exists(args),

        // math
        MATH_ABS => math::abs(args),
        MATH_SQRT => math::sqrt(args),
        MATH_POW => math::pow(args),
        MATH_FLOOR => math::floor(args),
        MATH_CEIL => math::ceil(args),
        MATH_ROUND => math::round(args),
        MATH_MIN => math::min(args),
        MATH_MAX => math::max(args),
        MATH_SIN => math::sin(args),
        MATH_COS => math::cos(args),
        MATH_TAN => math::tan(args),
        MATH_LOG => math::log(args),
        MATH_EXP => math::exp(args),
        RANDOM_NEXT => math::random_next(args),
        RANDOM_NEXT_DOUBLE => math::random_next_double(args),

        DT_NOW => time::now(false),
        DT_UTC_NOW => time::now(true),
        DT_TO_STRING => time::dt_to_string(args),

        JSON_PARSE | JSON_DESERIALIZE => json::parse(args),
        JSON_STRINGIFY | JSON_SERIALIZE => json::stringify(args),

        YAML_PARSE | YAML_DESERIALIZE => yaml::parse(args),
        YAML_SERIALIZE => yaml::serialize(args),

        HASH_SHA256 => crypto::sha256(args),
        HASH_SHA1 => crypto::sha1(args),
        HASH_MD5 => crypto::md5(args),

        HTTP_GET | HTTP_GET_ASYNC => net::http_get(args),
        HTTP_POST => net::http_post(args),
        TCP_CONNECT => net::tcp_connect(args),
        TCP_SEND => net::tcp_send(args),
        TCP_RECEIVE => net::tcp_receive(args),
        TCP_CLOSE => net::tcp_close(args),
        UDP_SEND => net::udp_send(args),
        UDP_RECEIVE => net::udp_receive(args),

        TASK_DELAY => {
            // Prefer VM path; sync fallback sleeps then returns ready Task
            let ms = args
                .iter()
                .rev()
                .find_map(|v| v.as_int().ok())
                .unwrap_or(0)
                .max(0) as u64;
            std::thread::sleep(std::time::Duration::from_millis(ms));
            Ok(Value::Task(crate::async_rt::TaskInner::new_ready(
                Value::Null,
            )))
        }
        TASK_RUN => {
            // Prefer VM path
            Ok(Value::Task(crate::async_rt::TaskInner::new_ready(
                Value::Null,
            )))
        }
        TASK_WHEN_ALL => {
            Ok(Value::Task(crate::async_rt::TaskInner::new_ready(
                crate::gc::alloc_array(Vec::new()),
            )))
        }
        GC_COLLECT | GC_STATS => Ok(Value::Null),

        LOG_INFO => logging::log("INFO", args),
        LOG_WARN => logging::log("WARN", args),
        LOG_ERROR => logging::log("ERROR", args),
        LOG_DEBUG => logging::log("DEBUG", args),

        _ => Err(crate::error::RuntimeError::Message(format!(
            "unknown native #{}",
            id
        ))),
    }
}

/// Create an empty collection object used by the compiler for `new Set/Queue/Stack`.
pub fn new_collection(kind: &str) -> Value {
    let mut fields = HashMap::new();
    fields.insert(
        "items".into(),
        crate::gc::alloc_array(Vec::new()),
    );
    crate::gc::alloc_object(ObjectInstance {
        class_name: kind.into(),
        fields,
        class_index: None,
        finalized: false,
    })
}

pub fn new_string_builder() -> Value {
    let mut fields = HashMap::new();
    fields.insert("buf".into(), Value::String("".into()));
    crate::gc::alloc_object(ObjectInstance {
        class_name: "StringBuilder".into(),
        fields,
        class_index: None,
        finalized: false,
    })
}

pub fn new_dict() -> Value {
    crate::gc::alloc_dict(HashMap::new())
}

pub fn new_logger() -> Value {
    crate::gc::alloc_object(ObjectInstance {
        class_name: "Logger".into(),
        fields: HashMap::new(),
        class_index: None,
        finalized: false,
    })
}

pub fn new_tcp_client() -> Value {
    let mut fields = HashMap::new();
    fields.insert("fd".into(), Value::Int(-1));
    crate::gc::alloc_object(ObjectInstance {
        class_name: "TcpClient".into(),
        fields,
        class_index: None,
        finalized: false,
    })
}

pub fn new_udp_socket(port: i64) -> Value {
    let mut fields = HashMap::new();
    fields.insert("port".into(), Value::Int(port));
    fields.insert("fd".into(), Value::Int(-1));
    crate::gc::alloc_object(ObjectInstance {
        class_name: "UdpSocket".into(),
        fields,
        class_index: None,
        finalized: false,
    })
}
