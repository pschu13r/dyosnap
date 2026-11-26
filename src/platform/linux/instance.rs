//! One DyoSnap at a time. The first instance owns the session bus name `APP_ID` and
//! serves the standard `org.freedesktop.Application` interface on it. A second start
//! calls that interface and exits, so there is one window and one set of shortcuts.

use super::{APP_ID, remember_activation_token};
use crate::platform::Sink;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::zvariant::{OwnedValue, Value};

const PATH: &str = "/io/github/pschu13r/DyoSnap";
const IFACE: &str = "org.freedesktop.Application";

type Data = HashMap<String, OwnedValue>;

/// Menu ids from other starts. They wait here until the app has set up its sink.
struct Inbox {
    sink: Option<Sink>,
    queue: Vec<String>,
}

static INBOX: Mutex<Inbox> = Mutex::new(Inbox { sink: None, queue: Vec::new() });
/// Keeps the bus name for as long as the process lives.
static CONN: OnceLock<zbus::Connection> = OnceLock::new();

/// Hands the calls of later starts to the app (same ids as the tray menu), also the ones
/// that came in before this.
pub fn serve_instance(sink: Sink) {
    let Ok(mut inbox) = INBOX.lock() else { return };
    for id in inbox.queue.drain(..) {
        sink(id);
    }
    inbox.sink = Some(sink);
}

fn deliver(id: String) {
    let Ok(mut inbox) = INBOX.lock() else { return };
    match &inbox.sink {
        Some(sink) => sink(id),
        None => inbox.queue.push(id),
    }
}

struct Service;

#[zbus::interface(name = "org.freedesktop.Application")]
impl Service {
    fn activate(&self, platform_data: Data) {
        keep_token(&platform_data);
        deliver("show".into());
    }

    fn open(&self, uris: Vec<String>, platform_data: Data) {
        keep_token(&platform_data);
        // Like the command line: the first file that exists, as an absolute path.
        let file = uris.iter().filter_map(|u| uri_to_path(u)).find(|p| p.is_file());
        match file {
            Some(p) => {
                let p = std::fs::canonicalize(&p).unwrap_or(p);
                deliver(format!("recent:{}", p.display()));
            }
            None => deliver("show".into()),
        }
    }

    fn activate_action(&self, name: String, parameter: Vec<OwnedValue>, platform_data: Data) {
        let _ = (name, parameter, platform_data);
    }
}

/// The activation token of the other start, so the window can come to the front.
fn keep_token(data: &Data) {
    let token = data.get("activation-token").and_then(|v| <&str>::try_from(&**v).ok());
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        remember_activation_token(t.to_string());
    }
}

/// Returns true if another DyoSnap runs and was told what to do: the caller exits.
/// Otherwise this process owns the bus name now. Without a session bus, or if the other
/// instance does not answer, it returns false and the app just starts.
pub fn single_instance(open: Option<&Path>, background: bool) -> bool {
    let r: zbus::Result<bool> = pollster::block_on(async {
        let conn = zbus::Connection::session().await?;
        conn.object_server().at(PATH, Service).await?;
        // zbus reports a name that another process owns as the error `NameTaken`.
        match conn.request_name_with_flags(APP_ID, RequestNameFlags::DO_NOT_QUEUE).await {
            Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {
                let _ = CONN.set(conn);
                return Ok(false);
            }
            Ok(_) | Err(zbus::Error::NameTaken) => {}
            Err(e) => return Err(e),
        }
        // "Launch at login" while it runs: nothing to show.
        if background {
            return Ok(true);
        }
        let mut data: HashMap<&str, Value> = HashMap::new();
        let token = std::env::var("XDG_ACTIVATION_TOKEN").or_else(|_| std::env::var("DESKTOP_STARTUP_ID"));
        if let Some(t) = token.ok().filter(|t| !t.is_empty()) {
            data.insert("activation-token", Value::from(t));
        }
        match open {
            Some(p) => {
                let uris = vec![path_to_uri(p)];
                conn.call_method(Some(APP_ID), PATH, Some(IFACE), "Open", &(uris, data)).await?;
            }
            None => {
                conn.call_method(Some(APP_ID), PATH, Some(IFACE), "Activate", &(data,)).await?;
            }
        }
        Ok(true)
    });
    r.unwrap_or_else(|e| {
        log::warn!(target: "dyosnap", "single instance check: {e}");
        false
    })
}

/// `file:///a/b%20c.png` to a path. Other schemes give None.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // An optional host ("localhost") sits before the first slash.
    let rest = &rest[rest.find('/')?..];
    let b = rest.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && b.len() >= i + 3 {
            if let Ok(v) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8(out).ok()?))
}

fn path_to_uri(p: &Path) -> String {
    let mut out = String::from("file://");
    for &c in p.to_string_lossy().as_bytes() {
        if c.is_ascii_alphanumeric() || b"/-_.~".contains(&c) {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}
