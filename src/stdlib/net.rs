//! bstd.net natives (synchronous).

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::{ObjectInstance, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::rc::Rc;
use std::time::Duration;

fn skip_module(args: &[Value]) -> &[Value] {
    if matches!(args.first(), Some(Value::TypeModule(_))) {
        &args[1..]
    } else {
        args
    }
}

thread_local! {
    static TCP_STREAMS: RefCell<HashMap<i64, TcpStream>> = RefCell::new(HashMap::new());
    static UDP_SOCKETS: RefCell<HashMap<i64, UdpSocket>> = RefCell::new(HashMap::new());
    static NEXT_FD: RefCell<i64> = RefCell::new(1);
}

fn next_fd() -> i64 {
    NEXT_FD.with(|n| {
        let mut n = n.borrow_mut();
        let id = *n;
        *n += 1;
        id
    })
}

pub fn http_get(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let url = args.first().map(|v| v.as_string()).unwrap_or_default();
    let resp = ureq::get(&url)
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| RuntimeError::Message(format!("HTTP GET failed: {}", e)))?;
    let status = resp.status();
    let body = resp
        .into_string()
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    let mut fields = HashMap::new();
    fields.insert("Status".into(), Value::Int(status as i64));
    fields.insert("Body".into(), Value::String(body.into()));
    Ok(crate::gc::alloc_object(ObjectInstance {
        class_name: "HttpResponse".into(),
        fields,
        class_index: None,
        finalized: false,
    }))
}

pub fn http_post(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let url = args.first().map(|v| v.as_string()).unwrap_or_default();
    let body = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    let content_type = args
        .get(2)
        .map(|v| v.as_string())
        .unwrap_or_else(|| "application/json".into());
    let resp = ureq::post(&url)
        .set("Content-Type", &content_type)
        .timeout(Duration::from_secs(30))
        .send_string(&body)
        .map_err(|e| RuntimeError::Message(format!("HTTP POST failed: {}", e)))?;
    let status = resp.status();
    let resp_body = resp
        .into_string()
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    let mut fields = HashMap::new();
    fields.insert("Status".into(), Value::Int(status as i64));
    fields.insert("Body".into(), Value::String(resp_body.into()));
    Ok(crate::gc::alloc_object(ObjectInstance {
        class_name: "HttpResponse".into(),
        fields,
        class_index: None,
        finalized: false,
    }))
}

pub fn tcp_connect(args: &[Value]) -> RuntimeResult<Value> {
    let (host, port) = match args.first() {
        Some(Value::Object(o)) => {
            let host = args.get(1).map(|v| v.as_string()).unwrap_or_else(|| "127.0.0.1".into());
            let port = args.get(2).map(|v| v.as_int()).transpose()?.unwrap_or(80);
            (host, port)
        }
        _ => {
            let host = args.first().map(|v| v.as_string()).unwrap_or_default();
            let port = args.get(1).map(|v| v.as_int()).transpose()?.unwrap_or(80);
            (host, port)
        }
    };
    let addr = format!("{}:{}", host, port);
    let stream = TcpStream::connect(&addr)
        .map_err(|e| RuntimeError::Message(format!("TCP connect {}: {}", addr, e)))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let fd = next_fd();
    TCP_STREAMS.with(|m| m.borrow_mut().insert(fd, stream));
    if let Some(Value::Object(o)) = args.first() {
        o.borrow_mut().fields.insert("fd".into(), Value::Int(fd));
        return Ok(Value::Null);
    }
    Ok(Value::Int(fd))
}

fn tcp_fd(args: &[Value]) -> RuntimeResult<i64> {
    match args.first() {
        Some(Value::Object(o)) => o
            .borrow()
            .fields
            .get("fd")
            .map(|v| v.as_int())
            .transpose()?
            .ok_or_else(|| RuntimeError::Message("TcpClient not connected".into())),
        Some(Value::Int(n)) => Ok(*n),
        _ => Err(RuntimeError::TypeError("expected TcpClient".into())),
    }
}

pub fn tcp_send(args: &[Value]) -> RuntimeResult<Value> {
    let fd = tcp_fd(args)?;
    let data = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    TCP_STREAMS.with(|m| {
        let mut map = m.borrow_mut();
        let stream = map
            .get_mut(&fd)
            .ok_or_else(|| RuntimeError::Message("invalid TCP fd".into()))?;
        stream
            .write_all(data.as_bytes())
            .map_err(|e| RuntimeError::Message(e.to_string()))?;
        Ok(Value::Null)
    })
}

pub fn tcp_receive(args: &[Value]) -> RuntimeResult<Value> {
    let fd = tcp_fd(args)?;
    TCP_STREAMS.with(|m| {
        let mut map = m.borrow_mut();
        let stream = map
            .get_mut(&fd)
            .ok_or_else(|| RuntimeError::Message("invalid TCP fd".into()))?;
        let mut buf = [0u8; 4096];
        let n = stream
            .read(&mut buf)
            .map_err(|e| RuntimeError::Message(e.to_string()))?;
        Ok(Value::String(
            String::from_utf8_lossy(&buf[..n]).into_owned().into(),
        ))
    })
}

pub fn tcp_close(args: &[Value]) -> RuntimeResult<Value> {
    if let Ok(fd) = tcp_fd(args) {
        TCP_STREAMS.with(|m| {
            m.borrow_mut().remove(&fd);
        });
        if let Some(Value::Object(o)) = args.first() {
            o.borrow_mut().fields.insert("fd".into(), Value::Int(-1));
        }
    }
    Ok(Value::Null)
}

pub fn udp_send(args: &[Value]) -> RuntimeResult<Value> {
    let (port, data, host, dest_port) = match args.first() {
        Some(Value::Object(o)) => {
            let port = o
                .borrow()
                .fields
                .get("port")
                .map(|v| v.as_int())
                .transpose()?
                .unwrap_or(0);
            let data = args.get(1).map(|v| v.as_string()).unwrap_or_default();
            let host = args.get(2).map(|v| v.as_string()).unwrap_or_else(|| "127.0.0.1".into());
            let dest = args.get(3).map(|v| v.as_int()).transpose()?.unwrap_or(0);
            (port, data, host, dest)
        }
        _ => {
            return Err(RuntimeError::TypeError("expected UdpSocket".into()));
        }
    };
    let sock = UdpSocket::bind(("0.0.0.0", port as u16))
        .or_else(|_| UdpSocket::bind(("0.0.0.0", 0)))
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    sock.send_to(data.as_bytes(), (host.as_str(), dest_port as u16))
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    Ok(Value::Null)
}

pub fn udp_receive(args: &[Value]) -> RuntimeResult<Value> {
    let port = match args.first() {
        Some(Value::Object(o)) => o
            .borrow()
            .fields
            .get("port")
            .map(|v| v.as_int())
            .transpose()?
            .unwrap_or(0),
        _ => 0,
    };
    let sock = UdpSocket::bind(("0.0.0.0", port as u16))
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = [0u8; 4096];
    let (n, addr) = sock
        .recv_from(&mut buf)
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    let mut fields = HashMap::new();
    fields.insert(
        "Data".into(),
        Value::String(String::from_utf8_lossy(&buf[..n]).into_owned().into()),
    );
    fields.insert("Ip".into(), Value::String(addr.ip().to_string().into()));
    fields.insert("Port".into(), Value::Int(addr.port() as i64));
    Ok(crate::gc::alloc_object(ObjectInstance {
        class_name: "UdpPacket".into(),
        fields,
        class_index: None,
        finalized: false,
    }))
}
