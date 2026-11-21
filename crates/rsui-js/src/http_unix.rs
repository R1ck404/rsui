//! Network requests outside Windows. Windows has WinHTTP for TLS, proxies and redirects (`http.rs`); Linux's will be
//! libcurl, loaded when first wanted, and macOS's `NSURLSession`. Until those are written, each request fails as one
//! with no network would, so `fetch` rejects and programs that can do without it carry on.

use crate::background::{Native, Outcome};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

fn not_here() -> Outcome {
    Outcome::NetFailed { code: "ENOSYS", message: "network requests aren't written for this system yet".to_owned() }
}

fn refuse(vm: &mut Vm, _: Args) -> Run<Value> {
    Ok(vm.settled_promise(not_here()))
}

fn nothing(_: &mut Vm, _: Args) -> Run<Value> {
    Ok(Value::UNDEFINED)
}

pub(crate) const NATIVES: [Native; 8] = [
    ("httpStart", 4, refuse),
    ("httpRead", 1, refuse),
    ("httpCancel", 1, nothing),
    ("wsConnect", 2, refuse),
    ("wsSend", 2, refuse),
    ("wsReceive", 1, refuse),
    ("wsClose", 3, nothing),
    ("wsForget", 1, nothing),
];
