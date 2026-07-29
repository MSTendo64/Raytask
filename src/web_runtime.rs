use crate::error::{RuntimeError, RuntimeResult};
use crate::value::{ObjectInstance, Value};
use regex::Regex;
use rusqlite::Connection;
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tiny_http::{Header, Method, Response, Server, StatusCode};

#[derive(Clone)]
struct WebRequestData {
    method: String,
    path: String,
    query: HashMap<String, String>,
    form: HashMap<String, String>,
    headers: HashMap<String, String>,
    cookies: HashMap<String, String>,
    body: String,
    script_dir: PathBuf,
    static_dir: Option<PathBuf>,
}

#[derive(Clone, Default)]
struct WebResponseData {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    handled: bool,
}

#[derive(Clone)]
struct WebContext {
    request: WebRequestData,
    response: WebResponseData,
}

thread_local! {
    static WEB_CONTEXT: RefCell<Option<WebContext>> = const { RefCell::new(None) };
    static SQLITE_NEXT_ID: RefCell<i64> = const { RefCell::new(1) };
    static SQLITE_CONNECTIONS: RefCell<HashMap<i64, Connection>> = RefCell::new(HashMap::new());
}

fn skip_module(args: &[Value]) -> &[Value] {
    if matches!(args.first(), Some(Value::TypeModule(_))) {
        &args[1..]
    } else {
        args
    }
}

fn next_sqlite_id() -> i64 {
    SQLITE_NEXT_ID.with(|n| {
        let mut n = n.borrow_mut();
        let id = *n;
        *n += 1;
        id
    })
}

fn header(name: &str, value: &str) -> RuntimeResult<Header> {
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .map_err(|_| RuntimeError::Message(format!("invalid header {}={}", name, value)))
}

fn lower_map(headers: &tiny_http::Header) -> (String, String) {
    (
        headers.field.as_str().to_ascii_lowercase().to_string(),
        headers.value.as_str().to_string(),
    )
}

fn parse_pairs(raw: &str) -> HashMap<String, String> {
    serde_urlencoded::from_str(raw).unwrap_or_default()
}

fn parse_cookies(raw: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for part in raw.split(';') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

fn with_ctx<R>(f: impl FnOnce(&WebContext) -> RuntimeResult<R>) -> RuntimeResult<R> {
    WEB_CONTEXT.with(|ctx| {
        let ctx = ctx.borrow();
        let ctx = ctx
            .as_ref()
            .ok_or_else(|| RuntimeError::Message("web context is not active".into()))?;
        f(ctx)
    })
}

fn with_ctx_mut<R>(f: impl FnOnce(&mut WebContext) -> RuntimeResult<R>) -> RuntimeResult<R> {
    WEB_CONTEXT.with(|ctx| {
        let mut ctx = ctx.borrow_mut();
        let ctx = ctx
            .as_mut()
            .ok_or_else(|| RuntimeError::Message("web context is not active".into()))?;
        f(ctx)
    })
}

fn set_default_content_type(ctx: &mut WebContext, value: &str) {
    if !ctx
        .response
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
        ctx.response
            .headers
            .push(("Content-Type".into(), value.into()));
    }
}

fn ensure_status(ctx: &mut WebContext) {
    if ctx.response.status == 0 {
        ctx.response.status = 200;
    }
}

fn json_to_rt(value: &JsonValue) -> Value {
    match value {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(v) => Value::Bool(*v),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else {
                Value::Float(n.as_f64().unwrap_or_default())
            }
        }
        JsonValue::String(s) => Value::String(s.clone().into()),
        JsonValue::Array(items) => {
            let arr = items.iter().map(json_to_rt).collect::<Vec<_>>();
            crate::gc::alloc_array(arr)
        }
        JsonValue::Object(map) => {
            let mut out = HashMap::new();
            for (k, v) in map {
                out.insert(k.clone(), json_to_rt(v));
            }
            crate::gc::alloc_dict(out)
        }
    }
}

fn rt_to_json(value: &Value) -> JsonValue {
    match value {
        Value::Null => JsonValue::Null,
        Value::Bool(v) => JsonValue::Bool(*v),
        Value::Int(v) => JsonValue::from(*v),
        Value::UInt(v) => JsonValue::from(*v),
        Value::Float(v) => JsonValue::from(*v),
        Value::Char(v) => JsonValue::String(v.to_string()),
        Value::String(v) => JsonValue::String(v.to_string()),
        Value::Array(arr) => JsonValue::Array(arr.borrow().iter().map(rt_to_json).collect()),
        Value::Dict(map) => {
            let mut obj = JsonMap::new();
            for (k, v) in map.borrow().iter() {
                obj.insert(k.clone(), rt_to_json(v));
            }
            JsonValue::Object(obj)
        }
        Value::Object(obj) => {
            let obj = obj.borrow();
            let mut out = JsonMap::new();
            out.insert("_class".into(), JsonValue::String(obj.class_name.clone()));
            for (k, v) in &obj.fields {
                out.insert(k.clone(), rt_to_json(v));
            }
            JsonValue::Object(out)
        }
        _ => JsonValue::String(value.as_string()),
    }
}

fn lookup_json_path<'a>(root: &'a JsonValue, path: &str) -> JsonValue {
    if path == "this" {
        return root.clone();
    }
    let mut current = root;
    for part in path.split('.') {
        let key = if part == "this" { continue } else { part };
        match current {
            JsonValue::Object(map) => {
                if let Some(next) = map.get(key) {
                    current = next;
                } else {
                    return JsonValue::Null;
                }
            }
            _ => return JsonValue::Null,
        }
    }
    current.clone()
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn render_template_text(input: &str, model: &JsonValue, root_dir: &Path) -> String {
    let include_re = Regex::new(r"\{\{\>\s*([a-zA-Z0-9_\-./]+)\s*\}\}").unwrap();
    let if_re = Regex::new(r"(?s)\{\{#if\s+([a-zA-Z0-9_.]+)\}\}(.*?)\{\{/if\}\}").unwrap();
    let each_re = Regex::new(r"(?s)\{\{#each\s+([a-zA-Z0-9_.]+)\}\}(.*?)\{\{/each\}\}").unwrap();
    let raw_value_re = Regex::new(r"\{\{\{\s*([a-zA-Z0-9_.]+)\s*\}\}\}").unwrap();
    let value_re = Regex::new(r"\{\{\s*([a-zA-Z0-9_.]+)\s*\}\}").unwrap();

    let mut text = input.to_string();

    loop {
        let Some(caps) = include_re.captures(&text) else {
            break;
        };
        let whole = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
        let rel = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let include_path = root_dir.join(rel);
        let rendered = fs::read_to_string(&include_path)
            .map(|raw| render_template_text(&raw, model, root_dir))
            .unwrap_or_else(|_| format!("<!-- missing include {} -->", include_path.display()));
        text = text.replacen(whole, &rendered, 1);
    }

    loop {
        let Some(caps) = if_re.captures(&text) else {
            break;
        };
        let whole = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
        let key = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let body = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
        let value = lookup_json_path(model, key);
        let rendered = if value.is_null()
            || matches!(value, JsonValue::Bool(false))
            || matches!(&value, JsonValue::String(s) if s.is_empty())
        {
            String::new()
        } else {
            render_template_text(body, model, root_dir)
        };
        text = text.replacen(whole, &rendered, 1);
    }

    loop {
        let Some(caps) = each_re.captures(&text) else {
            break;
        };
        let whole = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
        let key = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let body = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
        let value = lookup_json_path(model, key);
        let mut rendered = String::new();
        if let JsonValue::Array(items) = value {
            for item in items {
                let child = match item {
                    JsonValue::Object(mut map) => {
                        map.insert("this".into(), JsonValue::Object(map.clone()));
                        JsonValue::Object(map)
                    }
                    other => {
                        let mut map = JsonMap::new();
                        map.insert("this".into(), other);
                        JsonValue::Object(map)
                    }
                };
                rendered.push_str(&render_template_text(body, &child, root_dir));
            }
        }
        text = text.replacen(whole, &rendered, 1);
    }

    let text = raw_value_re
        .replace_all(&text, |caps: &regex::Captures| {
            let key = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let value = lookup_json_path(model, key);
            match value {
                JsonValue::Null => String::new(),
                JsonValue::String(s) => s,
                other => other.to_string(),
            }
        })
        .to_string();

    value_re
        .replace_all(&text, |caps: &regex::Captures| {
            let key = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let value = lookup_json_path(model, key);
            match value {
                JsonValue::Null => String::new(),
                JsonValue::String(s) => html_escape(&s),
                other => html_escape(&other.to_string()),
            }
        })
        .to_string()
}

fn render_template_file(path: &Path, model: &JsonValue) -> RuntimeResult<String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| RuntimeError::Message(format!("template read failed {}: {}", path.display(), e)))?;
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(render_template_text(&raw, model, root))
}

fn ext_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "html" | "htm" => "text/html; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn response_from_context(ctx: WebContext) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut response = Response::from_data(ctx.response.body).with_status_code(StatusCode(
        if ctx.response.status == 0 {
            200
        } else {
            ctx.response.status
        },
    ));
    for (name, value) in ctx.response.headers {
        if let Ok(header) = header(&name, &value) {
            response = response.with_header(header);
        }
    }
    response
}

pub fn http_server_serve_script(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let host = args.first().map(|v| v.as_string()).unwrap_or_else(|| "127.0.0.1".into());
    let port = args.get(1).map(|v| v.as_int()).transpose()?.unwrap_or(8080);
    let script = args.get(2).map(|v| v.as_string()).unwrap_or_default();
    let static_dir = args.get(3).map(|v| v.as_string()).filter(|s| !s.is_empty());
    let addr = format!("{}:{}", host, port);
    let server = Server::http(&addr)
        .map_err(|e| RuntimeError::Message(format!("http server bind {} failed: {}", addr, e)))?;
    let script_path = PathBuf::from(script);
    let static_dir = static_dir.map(PathBuf::from);

    println!("RayTask registry server listening on http://{}", addr);

    for mut request in server.incoming_requests() {
        let method = match request.method() {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Patch => "PATCH",
            Method::Head => "HEAD",
            Method::Options => "OPTIONS",
            _ => "GET",
        }
        .to_string();
        let url = request.url().to_string();
        let (path, query_raw) = match url.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (url.clone(), String::new()),
        };

        if let Some(static_root) = &static_dir {
            if path.starts_with("/static/") {
                let rel = path.trim_start_matches("/static/");
                let asset = static_root.join(rel);
                let result = if asset.is_file() {
                    match fs::read(&asset) {
                        Ok(bytes) => {
                            let mut response = Response::from_data(bytes)
                                .with_status_code(StatusCode(200));
                            if let Ok(ct) = header("Content-Type", ext_content_type(&asset)) {
                                response = response.with_header(ct);
                            }
                            if let Ok(cache) =
                                header("Cache-Control", "public, max-age=300")
                            {
                                response = response.with_header(cache);
                            }
                            response
                        }
                        Err(err) => Response::from_string(format!("asset read failed: {}", err))
                            .with_status_code(StatusCode(500)),
                    }
                } else {
                    Response::from_string("not found").with_status_code(StatusCode(404))
                };
                let _ = request.respond(result);
                continue;
            }
        }

        let mut body_bytes = Vec::new();
        if let Err(err) = request.as_reader().read_to_end(&mut body_bytes) {
            let _ = request.respond(
                Response::from_string(format!("bad request body: {}", err))
                    .with_status_code(StatusCode(400)),
            );
            continue;
        }
        let body = String::from_utf8_lossy(&body_bytes).into_owned();

        let mut headers = HashMap::new();
        for h in request.headers() {
            let (name, value) = lower_map(h);
            headers.insert(name, value);
        }
        let cookies = headers
            .get("cookie")
            .map(|v| parse_cookies(v))
            .unwrap_or_default();
        let query = parse_pairs(&query_raw);
        let form = if headers
            .get("content-type")
            .map(|v| v.contains("application/x-www-form-urlencoded"))
            .unwrap_or(false)
        {
            parse_pairs(&body)
        } else {
            HashMap::new()
        };

        let ctx = WebContext {
            request: WebRequestData {
                method,
                path,
                query,
                form,
                headers,
                cookies,
                body,
                script_dir: script_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf(),
                static_dir: static_dir.clone(),
            },
            response: WebResponseData {
                status: 200,
                headers: vec![],
                body: vec![],
                handled: false,
            },
        };

        WEB_CONTEXT.with(|cell| *cell.borrow_mut() = Some(ctx));
        let run = crate::run_file_with(
            &script_path,
            &crate::RunOptions {
                no_typecheck: true,
                ..crate::RunOptions::default()
            },
        );
        let ctx = WEB_CONTEXT.with(|cell| cell.borrow_mut().take());

        let response = match (run, ctx) {
            (Ok(_), Some(ctx)) => response_from_context(ctx),
            (Err(err), _) => Response::from_string(format!("RayTask request failed: {}", err))
                .with_status_code(StatusCode(500)),
            _ => Response::from_string("web context lost").with_status_code(StatusCode(500)),
        };
        let _ = request.respond(response);
    }

    Ok(Value::Null)
}

pub fn web_method(_: &[Value]) -> RuntimeResult<Value> {
    with_ctx(|ctx| Ok(Value::String(ctx.request.method.clone().into())))
}

pub fn web_path(_: &[Value]) -> RuntimeResult<Value> {
    with_ctx(|ctx| Ok(Value::String(ctx.request.path.clone().into())))
}

pub fn web_body(_: &[Value]) -> RuntimeResult<Value> {
    with_ctx(|ctx| Ok(Value::String(ctx.request.body.clone().into())))
}

pub fn web_header(args: &[Value]) -> RuntimeResult<Value> {
    let key = skip_module(args)
        .first()
        .map(|v| v.as_string().to_ascii_lowercase())
        .unwrap_or_default();
    with_ctx(|ctx| {
        Ok(Value::String(
            ctx.request
                .headers
                .get(&key)
                .cloned()
                .unwrap_or_default()
                .into(),
        ))
    })
}

pub fn web_query(args: &[Value]) -> RuntimeResult<Value> {
    let key = skip_module(args)
        .first()
        .map(|v| v.as_string())
        .unwrap_or_default();
    with_ctx(|ctx| {
        Ok(Value::String(
            ctx.request.query.get(&key).cloned().unwrap_or_default().into(),
        ))
    })
}

pub fn web_form(args: &[Value]) -> RuntimeResult<Value> {
    let key = skip_module(args)
        .first()
        .map(|v| v.as_string())
        .unwrap_or_default();
    with_ctx(|ctx| {
        Ok(Value::String(
            ctx.request.form.get(&key).cloned().unwrap_or_default().into(),
        ))
    })
}

pub fn web_cookie(args: &[Value]) -> RuntimeResult<Value> {
    let key = skip_module(args)
        .first()
        .map(|v| v.as_string())
        .unwrap_or_default();
    with_ctx(|ctx| {
        Ok(Value::String(
            ctx.request
                .cookies
                .get(&key)
                .cloned()
                .unwrap_or_default()
                .into(),
        ))
    })
}

pub fn web_is_htmx(_: &[Value]) -> RuntimeResult<Value> {
    with_ctx(|ctx| {
        Ok(Value::Bool(
            ctx.request
                .headers
                .get("hx-request")
                .map(|v| v == "true")
                .unwrap_or(false),
        ))
    })
}

pub fn web_script_dir(_: &[Value]) -> RuntimeResult<Value> {
    with_ctx(|ctx| Ok(Value::String(ctx.request.script_dir.display().to_string().into())))
}

pub fn web_static_dir(_: &[Value]) -> RuntimeResult<Value> {
    with_ctx(|ctx| {
        Ok(Value::String(
            ctx.request
                .static_dir
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
                .into(),
        ))
    })
}

pub fn web_set_status(args: &[Value]) -> RuntimeResult<Value> {
    let status = skip_module(args)
        .first()
        .map(|v| v.as_int())
        .transpose()?
        .unwrap_or(200) as u16;
    with_ctx_mut(|ctx| {
        ctx.response.status = status;
        Ok(Value::Null)
    })
}

pub fn web_set_header(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let name = args.first().map(|v| v.as_string()).unwrap_or_default();
    let value = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    with_ctx_mut(|ctx| {
        ctx.response.headers.push((name, value));
        Ok(Value::Null)
    })
}

pub fn web_set_cookie(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let name = args.first().map(|v| v.as_string()).unwrap_or_default();
    let value = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    let path = args
        .get(2)
        .map(|v| v.as_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/".into());
    let http_only = args.get(3).map(|v| v.is_truthy()).unwrap_or(true);
    let max_age = args.get(4).map(|v| v.as_int()).transpose()?.unwrap_or(0);
    let mut cookie = format!("{}={}; Path={}", name, value, path);
    if http_only {
        cookie.push_str("; HttpOnly");
    }
    if max_age != 0 {
        cookie.push_str(&format!("; Max-Age={}", max_age));
    }
    with_ctx_mut(|ctx| {
        ctx.response.headers.push(("Set-Cookie".into(), cookie));
        Ok(Value::Null)
    })
}

pub fn web_write(args: &[Value]) -> RuntimeResult<Value> {
    let text = skip_module(args)
        .first()
        .map(|v| v.as_string())
        .unwrap_or_default();
    with_ctx_mut(|ctx| {
        ensure_status(ctx);
        set_default_content_type(ctx, "text/html; charset=utf-8");
        ctx.response.body.extend_from_slice(text.as_bytes());
        ctx.response.handled = true;
        Ok(Value::Null)
    })
}

pub fn web_text(args: &[Value]) -> RuntimeResult<Value> {
    let text = skip_module(args)
        .first()
        .map(|v| v.as_string())
        .unwrap_or_default();
    with_ctx_mut(|ctx| {
        ensure_status(ctx);
        ctx.response
            .headers
            .push(("Content-Type".into(), "text/plain; charset=utf-8".into()));
        ctx.response.body = text.into_bytes();
        ctx.response.handled = true;
        Ok(Value::Null)
    })
}

pub fn web_html(args: &[Value]) -> RuntimeResult<Value> {
    let html = skip_module(args)
        .first()
        .map(|v| v.as_string())
        .unwrap_or_default();
    with_ctx_mut(|ctx| {
        ensure_status(ctx);
        ctx.response
            .headers
            .push(("Content-Type".into(), "text/html; charset=utf-8".into()));
        ctx.response.body = html.into_bytes();
        ctx.response.handled = true;
        Ok(Value::Null)
    })
}

pub fn web_json(args: &[Value]) -> RuntimeResult<Value> {
    let payload = skip_module(args).first().cloned().unwrap_or(Value::Null);
    let json = serde_json::to_string_pretty(&rt_to_json(&payload))
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    with_ctx_mut(|ctx| {
        ensure_status(ctx);
        ctx.response
            .headers
            .push(("Content-Type".into(), "application/json; charset=utf-8".into()));
        ctx.response.body = json.into_bytes();
        ctx.response.handled = true;
        Ok(Value::Null)
    })
}

pub fn web_redirect(args: &[Value]) -> RuntimeResult<Value> {
    let url = skip_module(args)
        .first()
        .map(|v| v.as_string())
        .unwrap_or_else(|| "/".into());
    with_ctx_mut(|ctx| {
        ctx.response.status = 302;
        ctx.response.headers.push(("Location".into(), url));
        ctx.response.handled = true;
        Ok(Value::Null)
    })
}

pub fn web_file(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let path = args.first().map(|v| v.as_string()).unwrap_or_default();
    let content_type = args
        .get(1)
        .map(|v| v.as_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ext_content_type(Path::new(&path)).to_string());
    let bytes = fs::read(&path)
        .map_err(|e| RuntimeError::Message(format!("file response read failed {}: {}", path, e)))?;
    with_ctx_mut(|ctx| {
        ensure_status(ctx);
        ctx.response
            .headers
            .push(("Content-Type".into(), content_type));
        ctx.response.body = bytes;
        ctx.response.handled = true;
        Ok(Value::Null)
    })
}

pub fn template_render(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let path = args.first().map(|v| v.as_string()).unwrap_or_default();
    let model = args.get(1).cloned().unwrap_or(Value::Null);
    let template = render_template_file(Path::new(&path), &rt_to_json(&model))?;
    Ok(Value::String(template.into()))
}

pub fn web_render(args: &[Value]) -> RuntimeResult<Value> {
    let rendered = template_render(args)?.as_string();
    with_ctx_mut(|ctx| {
        ensure_status(ctx);
        ctx.response
            .headers
            .push(("Content-Type".into(), "text/html; charset=utf-8".into()));
        ctx.response.body = rendered.into_bytes();
        ctx.response.handled = true;
        Ok(Value::Null)
    })
}

pub fn web_parse_json(args: &[Value]) -> RuntimeResult<Value> {
    let text = skip_module(args)
        .first()
        .map(|v| v.as_string())
        .unwrap_or_default();
    let value: JsonValue =
        serde_json::from_str(&text).map_err(|e| RuntimeError::Message(format!("json parse failed: {}", e)))?;
    Ok(json_to_rt(&value))
}

pub fn sqlite_open(args: &[Value]) -> RuntimeResult<Value> {
    let path = skip_module(args)
        .first()
        .map(|v| v.as_string())
        .unwrap_or_else(|| "registry.db".into());
    let conn = Connection::open(&path)
        .map_err(|e| RuntimeError::Message(format!("sqlite open failed {}: {}", path, e)))?;
    let id = next_sqlite_id();
    SQLITE_CONNECTIONS.with(|map| {
        map.borrow_mut().insert(id, conn);
    });
    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::Int(id));
    fields.insert("path".into(), Value::String(path.into()));
    Ok(crate::gc::alloc_object(ObjectInstance {
        class_name: "SqliteConnection".into(),
        fields,
        class_index: None,
        finalized: false,
    }))
}

fn sqlite_id(args: &[Value]) -> RuntimeResult<i64> {
    match args.first() {
        Some(Value::Object(obj)) => obj
            .borrow()
            .fields
            .get("id")
            .map(|v| v.as_int())
            .transpose()?
            .ok_or_else(|| RuntimeError::Message("sqlite connection is closed".into())),
        Some(Value::Int(id)) => Ok(*id),
        _ => Err(RuntimeError::TypeError("expected SqliteConnection".into())),
    }
}

fn with_conn<R>(id: i64, f: impl FnOnce(&Connection) -> RuntimeResult<R>) -> RuntimeResult<R> {
    SQLITE_CONNECTIONS.with(|map| {
        let map = map.borrow();
        let conn = map
            .get(&id)
            .ok_or_else(|| RuntimeError::Message(format!("sqlite connection {} not found", id)))?;
        f(conn)
    })
}

pub fn sqlite_execute(args: &[Value]) -> RuntimeResult<Value> {
    let id = sqlite_id(args)?;
    let sql = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    with_conn(id, |conn| {
        let changed = conn
            .execute_batch(&sql)
            .map(|_| 0_i64)
            .map_err(|e| RuntimeError::Message(format!("sqlite execute failed: {}", e)))?;
        Ok(Value::Int(changed))
    })
}

fn row_to_value(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<JsonValue> {
    use rusqlite::types::ValueRef;
    match row.get_ref(idx)? {
        ValueRef::Null => Ok(JsonValue::Null),
        ValueRef::Integer(v) => Ok(JsonValue::from(v)),
        ValueRef::Real(v) => Ok(JsonValue::from(v)),
        ValueRef::Text(v) => Ok(JsonValue::String(String::from_utf8_lossy(v).into_owned())),
        ValueRef::Blob(v) => Ok(JsonValue::String(hex::encode(v))),
    }
}

pub fn sqlite_query(args: &[Value]) -> RuntimeResult<Value> {
    let id = sqlite_id(args)?;
    let sql = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    with_conn(id, |conn| {
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| RuntimeError::Message(format!("sqlite prepare failed: {}", e)))?;
        let names = stmt.column_names().iter().map(|v| v.to_string()).collect::<Vec<_>>();
        let rows = stmt
            .query_map([], |row| {
                let mut map = JsonMap::new();
                for (idx, name) in names.iter().enumerate() {
                    map.insert(name.clone(), row_to_value(row, idx)?);
                }
                Ok(JsonValue::Object(map))
            })
            .map_err(|e| RuntimeError::Message(format!("sqlite query failed: {}", e)))?;
        let mut items = Vec::new();
        for row in rows {
            items.push(json_to_rt(&row.map_err(|e| RuntimeError::Message(e.to_string()))?));
        }
        Ok(crate::gc::alloc_array(items))
    })
}

pub fn sqlite_query_one(args: &[Value]) -> RuntimeResult<Value> {
    let rows = sqlite_query(args)?;
    match rows {
        Value::Array(arr) => Ok(arr.borrow().first().cloned().unwrap_or(Value::Null)),
        _ => Ok(Value::Null),
    }
}

pub fn sqlite_last_insert_rowid(args: &[Value]) -> RuntimeResult<Value> {
    let id = sqlite_id(args)?;
    with_conn(id, |conn| Ok(Value::Int(conn.last_insert_rowid())))
}

pub fn sqlite_close(args: &[Value]) -> RuntimeResult<Value> {
    let id = sqlite_id(args)?;
    SQLITE_CONNECTIONS.with(|map| {
        map.borrow_mut().remove(&id);
    });
    if let Some(Value::Object(obj)) = args.first() {
        obj.borrow_mut().fields.remove("id");
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_cookie_header_reads_pairs() {
        let cookies = parse_cookies("rt_session=abc123; theme=dark; spaced = ok");
        assert_eq!(cookies.get("rt_session").map(String::as_str), Some("abc123"));
        assert_eq!(cookies.get("theme").map(String::as_str), Some("dark"));
        assert_eq!(cookies.get("spaced").map(String::as_str), Some("ok"));
    }

    #[test]
    fn template_render_supports_raw_if_and_each() {
        let model = json!({
            "title": "Registry",
            "enabled": true,
            "content": "<b>raw</b>",
            "items": [
                { "name": "alpha" },
                { "name": "beta" }
            ]
        });
        let template = "\
<h1>{{title}}</h1>\
{{#if enabled}}<div>{{{content}}}</div>{{/if}}\
<ul>{{#each items}}<li>{{name}}</li>{{/each}}</ul>";
        let rendered = render_template_text(template, &model, Path::new("."));
        assert!(rendered.contains("<h1>Registry</h1>"));
        assert!(rendered.contains("<div><b>raw</b></div>"));
        assert!(rendered.contains("<li>alpha</li>"));
        assert!(rendered.contains("<li>beta</li>"));
    }
}
