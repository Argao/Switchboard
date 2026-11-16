//! The desktop UI drives the daemon over stdin/stdout, one JSON object per
//! line. Nothing listens on a port for this, so no other local process can
//! start a pairing or forget a phone.
//!
//! In:  `{"id": 1, "cmd": "pair_start"}`
//! Out: `{"id": 1, "ok": true, "data": ...}` and `{"event": "status", "data": ...}`

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

use switchboard_core::protocol::Command;

use crate::app::App;

// Arguments share the line with the request `id`, so no command may take
// an argument called `id`.
#[derive(Deserialize)]
struct Input {
    id: u64,
    #[serde(flatten)]
    cmd: Cmd,
}

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Cmd {
    Status,
    PairStart,
    PairCancel,
    Forget { device: String },
    Rename { name: String },
    /// Let phones use this computer's keyboard and mouse (off by default).
    SetRemoteInput { on: bool },
    /// What a phone would see right now, for the desktop's own dashboard.
    State,
    /// Offer files and folders (picked in the desktop's own dialog) to a
    /// connected phone. A folder is zipped first.
    SendFiles { device: String, paths: Vec<String> },
    /// Answer files phones are waiting to send: one, or all when `xfer` is absent.
    Approve { xfer: Option<String>, accept: bool },
    /// Take files phones send without asking.
    SetAutoAccept { on: bool },
    /// Recent transfers, newest first.
    History,
    /// The folder phones' uploads are saved to.
    Inbox,
    /// Use the smart plug at this address, with this local key, for total
    /// shutdowns. Answers with what the plug reports, having asked it.
    SetPlug { host: Option<String>, key: String },
    ForgetPlug,
    /// Whether a plug is set up, and what it reports right now.
    Plug,
    /// Run a control (brightness, volume, media) from the desktop itself.
    Run { command: Command },
}

/// The plug for the desktop's settings page. Never its key.
fn plug_view(state: Option<anyhow::Result<crate::system::tuya::PlugState>>) -> Value {
    match state {
        None => json!({ "configured": false }),
        Some(Ok(s)) => json!({ "configured": true, "reachable": true, "on": s.on, "watts": s.watts }),
        Some(Err(e)) => json!({ "configured": true, "reachable": false, "error": format!("{e:#}") }),
    }
}

async fn handle(app: &Arc<App>, cmd: Cmd) -> anyhow::Result<Value> {
    Ok(match cmd {
        Cmd::Status => app.status(),
        Cmd::PairStart => {
            app.start_pairing();
            app.emit_status();
            app.status()
        }
        Cmd::PairCancel => {
            app.cancel_pairing();
            app.emit_status();
            Value::Null
        }
        Cmd::Forget { device } => {
            let removed = app.forget(&device)?;
            app.emit_status();
            json!({ "removed": removed })
        }
        Cmd::SetRemoteInput { on } => {
            app.set_remote_input(on)?;
            app.emit_status();
            Value::Null
        }
        Cmd::Approve { xfer, accept } => json!({ "answered": app.answer_uploads(xfer.as_deref(), accept) }),
        Cmd::SetAutoAccept { on } => {
            app.set_auto_accept(on)?;
            app.emit_status();
            Value::Null
        }
        Cmd::Rename { name } => {
            app.rename(&name)?;
            app.emit_status();
            Value::Null
        }
        Cmd::SendFiles { device, paths } => {
            let app2 = app.clone();
            let sent = tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
                for p in &paths {
                    let p = std::path::Path::new(p);
                    if p.is_dir() {
                        let (zip, _) = crate::files::zip_dir(p)?;
                        app2.offer_file(&device, &zip, true)?;
                    } else {
                        app2.offer_file(&device, p, false)?;
                    }
                }
                Ok(paths.len())
            })
            .await??;
            json!({ "sent": sent })
        }
        Cmd::History => json!(app.store.transfers(50)?),
        Cmd::Inbox => {
            std::fs::create_dir_all(&app.inbox)?;
            json!(app.inbox)
        }
        Cmd::SetPlug { host, key } => {
            let s = app.set_plug(host.as_deref(), &key).await?;
            app.poke.notify_one();
            plug_view(Some(Ok(s)))
        }
        Cmd::ForgetPlug => {
            app.forget_plug()?;
            app.poke.notify_one();
            Value::Null
        }
        Cmd::Plug => {
            let mut v = match app.system.total.plug() {
                Some(p) => plug_view(Some(p.state().await)),
                None => plug_view(None),
            };
            // Where it is, for the settings page; never its key.
            if let Some((host, _)) = app.store.plug(&app.host_id)? {
                v["host"] = json!(host);
            }
            v
        }
        Cmd::State => serde_json::to_value(app.system.snapshot().await)?,
        Cmd::Run { command } => {
            let data = app.system.run(&command).await?;
            // Connected phones see the change on their next state push.
            app.poke.notify_one();
            json!(data)
        }
    })
}

/// Returns when stdin closes: the UI went away, so the daemon does too.
pub async fn run(app: Arc<App>) {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let input: Input = match serde_json::from_str(&line) {
            Ok(i) => i,
            Err(e) => {
                app.emit("error", json!(format!("bad control line: {e}")));
                continue;
            }
        };
        let slow = matches!(input.cmd, Cmd::State | Cmd::Run { .. } | Cmd::SendFiles { .. } | Cmd::SetPlug { .. } | Cmd::Plug);
        let app2 = app.clone();
        let reply = async move {
            let reply = match handle(&app2, input.cmd).await {
                Ok(data) => json!({ "id": input.id, "ok": true, "data": data }),
                Err(e) => json!({ "id": input.id, "ok": false, "error": format!("{e:#}") }),
            };
            let _ = app2.ui.send(reply.to_string());
        };
        // Only the slow hardware read runs aside; everything else keeps the
        // order it was sent in, so a start and a cancel cannot swap places.
        if slow {
            tokio::spawn(reply);
        } else {
            reply.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forget_line_keeps_request_id() {
        let i: Input = serde_json::from_str(r#"{"id":7,"cmd":"forget","device":"abc"}"#).unwrap();
        assert_eq!(i.id, 7);
        assert!(matches!(i.cmd, Cmd::Forget { device } if device == "abc"));
    }

    #[test]
    fn plug_lines_parse() {
        let i: Input = serde_json::from_str(r#"{"id":3,"cmd":"set_plug","host":"192.168.1.22","key":"0123456789abcdef"}"#).unwrap();
        assert!(matches!(i.cmd, Cmd::SetPlug { host: Some(host), key } if host == "192.168.1.22" && key.len() == 16));
        // Without an address the network is searched.
        let i: Input = serde_json::from_str(r#"{"id":3,"cmd":"set_plug","key":"0123456789abcdef"}"#).unwrap();
        assert!(matches!(i.cmd, Cmd::SetPlug { host: None, .. }));
        for cmd in ["plug", "forget_plug"] {
            assert!(serde_json::from_str::<Input>(&format!(r#"{{"id":5,"cmd":"{cmd}"}}"#)).is_ok(), "{cmd}");
        }
    }

    #[test]
    fn the_plug_is_shown_without_its_key() {
        use crate::system::tuya::PlugState;
        assert_eq!(plug_view(None), json!({ "configured": false }));
        let v = plug_view(Some(Ok(PlugState { on: true, countdown: 0, watts: Some(52.5) })));
        assert_eq!(v, json!({ "configured": true, "reachable": true, "on": true, "watts": 52.5 }));
        let v = plug_view(Some(Err(anyhow::anyhow!("no answer"))));
        assert_eq!(v, json!({ "configured": true, "reachable": false, "error": "no answer" }));
    }

    #[test]
    fn run_line_parses() {
        let i: Input = serde_json::from_str(r#"{"id":1,"cmd":"run","command":{"type":"set_volume","level":5}}"#).unwrap();
        assert!(matches!(i.cmd, Cmd::Run { .. }));
    }
}
