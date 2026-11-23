//! Where the windows are, for "Capture Window" on the frozen picture.
//!
//! Wayland tells no app where other windows are. KDE's KWin runs scripts on request
//! (org.kde.kwin.Scripting, open to every app): a small script lists the windows and sends
//! the list back to DyoSnap over D-Bus. Other desktops have no such way; there "Capture
//! Window" uses the desktop's own picker.

use egui::{Rect, pos2, vec2};
use std::sync::mpsc;
use std::time::Duration;

/// Whether this desktop can list its windows (KDE Plasma).
pub fn can_list_windows() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|p| p.eq_ignore_ascii_case("KDE")))
}

/// The visible windows of the current desktop, top-most first, in points on the desktop
/// (with their title bars). Empty when they can't be listed.
pub fn window_rects() -> Vec<Rect> {
    if !can_list_windows() {
        return Vec::new();
    }
    match pollster::block_on(list()) {
        Ok(v) => v,
        Err(e) => {
            log::warn!(target: "dyosnap", "window list: {e}");
            Vec::new()
        }
    }
}

const PATH: &str = "/io/github/pschu13r/DyoSnap/Windows";
const IFACE: &str = "io.github.pschu13r.DyoSnap.Windows";

struct Receiver(mpsc::SyncSender<String>);

#[zbus::interface(name = "io.github.pschu13r.DyoSnap.Windows")]
impl Receiver {
    fn report(&self, json: String) {
        let _ = self.0.try_send(json);
    }
}

#[derive(serde::Deserialize)]
struct Win {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

async fn list() -> Result<Vec<Rect>, String> {
    let conn = zbus::Connection::session().await.map_err(|e| e.to_string())?;
    let me = conn.unique_name().ok_or("no D-Bus name")?.to_string();
    let (tx, rx) = mpsc::sync_channel(1);
    conn.object_server().at(PATH, Receiver(tx)).await.map_err(|e| e.to_string())?;

    // Bottom to top, as KWin stacks them. Only normal windows and dialogs that are shown
    // on the current virtual desktop.
    let script = format!(
        r#"const out = [];
for (const w of workspace.stackingOrder) {{
  if (!(w.normalWindow || w.dialog) || w.minimized || w.hidden) continue;
  let here = true;
  try {{ here = w.onAllDesktops || w.desktops.indexOf(workspace.currentDesktop) >= 0; }} catch (e) {{}}
  if (!here) continue;
  const g = w.frameGeometry;
  out.push({{x: g.x, y: g.y, w: g.width, h: g.height}});
}}
callDBus("{me}", "{PATH}", "{IFACE}", "Report", JSON.stringify(out));
"#
    );
    run_script(&conn, "windows", &script).await?;
    // The answer comes as a call to us, served by the connection's own task.
    let json = blocking_recv(rx, Duration::from_secs(1));
    let json = json.ok_or("KWin did not answer")?;
    let wins: Vec<Win> = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    Ok(wins
        .into_iter()
        .rev()
        .filter(|w| w.w >= 1.0 && w.h >= 1.0)
        .map(|w| Rect::from_min_size(pos2(w.x, w.y), vec2(w.w, w.h)))
        .collect())
}

/// Runs a one-shot KWin script.
async fn run_script(conn: &zbus::Connection, what: &str, script: &str) -> Result<(), String> {
    let file = crate::platform::temp_file(what, "js");
    std::fs::write(&file, script).map_err(|e| e.to_string())?;
    let name = format!("dyosnap-{what}-{}", std::process::id());
    let scripting = zbus::Proxy::new(conn, "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting").await.map_err(|e| e.to_string())?;
    // A script of the same name left from before would block loading.
    let _: Result<bool, _> = scripting.call("unloadScript", &(name.as_str(),)).await;
    let id: i32 = scripting.call("loadScript", &(file.display().to_string(), name.as_str())).await.map_err(|e| e.to_string())?;
    let script_path = format!("/Scripting/Script{id}");
    let script_obj = zbus::Proxy::new(conn, "org.kde.KWin", script_path.as_str(), "org.kde.kwin.Script").await.map_err(|e| e.to_string())?;
    let run: Result<(), _> = script_obj.call("run", &()).await;
    let _: Result<bool, _> = scripting.call("unloadScript", &(name.as_str(),)).await;
    let _ = std::fs::remove_file(&file);
    run.map_err(|e| e.to_string())
}

/// KDE: puts DyoSnap's main window in front through a KWin script. For shortcuts, whose
/// portal sends no activation token on KDE. In the background; does nothing elsewhere.
pub fn kwin_activate_main() {
    if !can_list_windows() {
        return;
    }
    let script = format!(
        r#"let main = null;
for (const w of workspace.stackingOrder) {{
  if (w.pid === {pid} && w.normalWindow && !w.caption.startsWith("DyoSnap | ")) main = w;
}}
if (main) {{
  main.minimized = false;
  workspace.activeWindow = main;
}}
"#,
        pid = std::process::id()
    );
    std::thread::spawn(move || {
        let res = pollster::block_on(async {
            let conn = zbus::Connection::session().await.map_err(|e| e.to_string())?;
            run_script(&conn, "activate", &script).await
        });
        match res {
            Ok(()) => log::info!(target: "dyosnap", "main window activated through KWin"),
            Err(e) => log::warn!(target: "dyosnap", "KWin activation: {e}"),
        }
    });
}

/// Waits for the answer without blocking the async executor's other work for long.
fn blocking_recv(rx: mpsc::Receiver<String>, timeout: Duration) -> Option<String> {
    let end = std::time::Instant::now() + timeout;
    loop {
        if let Ok(s) = rx.try_recv() {
            return Some(s);
        }
        if std::time::Instant::now() > end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
