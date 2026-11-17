//! The machine the phone controls.

pub mod clipboard;
pub mod displays;
pub mod input;
pub mod media;
pub mod mixer;
pub mod power;
pub mod rgb;
pub mod stats;
pub mod toggles;
pub mod total;
pub mod tuya;
pub mod volume;
pub mod windows;

use anyhow::{Result, anyhow};
use switchboard_core::protocol::{Command, HostState};

#[derive(Default)]
pub struct System {
    pub displays: displays::Displays,
    pub rgb: rgb::Rgb,
    /// Shutting down and cutting the power at the smart plug.
    pub total: std::sync::Arc<total::Total>,
    stats: stats::Sampler,
    /// The player the user picked; `None` follows whichever is playing.
    player: std::sync::Mutex<Option<String>>,
    /// Where the cover of the track in the last snapshot lives.
    art_url: std::sync::Mutex<Option<String>>,
    clipboard: std::sync::Arc<clipboard::Clip>,
    /// Whether phones may use the keyboard and mouse; off until the person at the computer allows it.
    pub remote_input: std::sync::atomic::AtomicBool,
    /// Quick settings are read by running programs, so they are cached for a few seconds.
    toggles: std::sync::Mutex<Option<(std::time::Instant, Vec<switchboard_core::protocol::ToggleState>)>>,
}

const TOGGLES_FRESH: std::time::Duration = std::time::Duration::from_secs(5);

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| anyhow!("task failed: {e}"))
}

impl System {
    pub async fn snapshot(&self) -> HostState {
        let selected = self.player.lock().unwrap().clone();
        let (volume, outputs, apps, displays, now, toggles, open) = tokio::join!(
            async { blocking(volume::get).await.ok().and_then(Result::ok) },
            async { blocking(mixer::outputs).await.ok().and_then(Result::ok).unwrap_or_default() },
            async { blocking(mixer::apps).await.ok().and_then(Result::ok).unwrap_or_default() },
            self.displays.list(),
            media::now_playing(selected),
            self.toggles(),
            async { blocking(windows::list).await.ok().flatten() },
        );
        *self.art_url.lock().unwrap() = now.art_url;
        let (windows, screens) = open.map_or((None, Vec::new()), |(w, s)| (Some(w), s));
        HostState { volume, outputs, apps, displays, rgb: self.rgb.state(), media: now.media, players: now.players, toggles, remote_input: self.remote_input.load(std::sync::atomic::Ordering::Relaxed), windows, screens, streams: windows::streams(), total_shutdown: self.total.state(), stats: self.stats.sample() }
    }

    async fn toggles(&self) -> Vec<switchboard_core::protocol::ToggleState> {
        if let Some((at, list)) = self.toggles.lock().unwrap().as_ref() {
            if at.elapsed() < TOGGLES_FRESH {
                return list.clone();
            }
        }
        let list = blocking(toggles::list).await.unwrap_or_default();
        *self.toggles.lock().unwrap() = Some((std::time::Instant::now(), list.clone()));
        list
    }

    /// Runs one phone command; the result's `data`, if the command returns any.
    pub async fn run(&self, cmd: &Command) -> Result<Option<String>> {
        match cmd {
            Command::MediaArt => {
                let url = self.art_url.lock().unwrap().clone().ok_or_else(|| anyhow!("no cover"))?;
                let bytes = blocking(move || media::fetch_art(&url)).await??;
                Ok(Some(switchboard_core::b64(&bytes)))
            }
            Command::ClipboardGet => {
                let c = self.clipboard.clone();
                Ok(Some(blocking(move || c.get()).await??))
            }
            _ => self.run_unit(cmd).await.map(|()| None),
        }
    }

    /// `Hello`, `GetState` and `Ping` carry no action.
    async fn run_unit(&self, cmd: &Command) -> Result<()> {
        let selected = || self.player.lock().unwrap().clone();
        match cmd {
            // Unpair and the file commands are handled by the session, which knows the device.
            Command::Hello | Command::GetState | Command::Ping | Command::Unpair => Ok(()),
            Command::FileSend { .. } | Command::FileChunk { .. } | Command::FileDone { .. } | Command::FileCancel { .. } | Command::FileRead { .. } => {
                Err(anyhow!("file commands run in the session"))
            }
            Command::SetVolume { level } => {
                let l = *level;
                blocking(move || volume::set_level(l)).await?
            }
            Command::SetMute { muted } => {
                let m = *muted;
                blocking(move || volume::set_mute(m)).await?
            }
            Command::SetOutput { output } => {
                let o = output.clone();
                blocking(move || mixer::set_output(&o)).await?
            }
            Command::SetAppVolume { app, level } => {
                let (a, l) = (app.clone(), *level);
                blocking(move || mixer::set_app_volume(&a, l)).await?
            }
            Command::SetAppMute { app, muted } => {
                let (a, m) = (app.clone(), *muted);
                blocking(move || mixer::set_app_mute(&a, m)).await?
            }
            Command::SetBrightness { display, level } => self.displays.set(display, displays::VCP_BRIGHTNESS, *level).await,
            Command::SetContrast { display, level } => self.displays.set(display, displays::VCP_CONTRAST, *level).await,
            Command::SetDisplayPower { display, on } => self.displays.set_power(display, *on).await,
            Command::SetRgbPower { on } => self.rgb.change(|s| s.on = *on),
            Command::SetRgbBrightness { level } => self.rgb.change(|s| s.brightness = (*level).min(100)),
            Command::SetRgbEffect { effect } => self.rgb.change(|s| s.effect = *effect),
            Command::SetRgbColor { color } => {
                let c = rgb::parse_color(color).ok_or_else(|| anyhow!("not a colour: {color}"))?;
                self.rgb.change(|s| s.color = rgb::hex(c))
            }
            Command::SetRgbSpeed { level } => self.rgb.change(|s| s.speed = (*level).min(100)),
            Command::Media { action } => media::control(selected(), *action).await,
            Command::Seek { position } => media::seek(selected(), *position).await,
            Command::SetShuffle { on } => media::set_shuffle(selected(), *on).await,
            Command::SetRepeat { mode } => media::set_repeat(selected(), *mode).await,
            Command::SelectPlayer { player } => {
                *self.player.lock().unwrap() = Some(player.clone());
                Ok(())
            }
            Command::MediaArt | Command::ClipboardGet => Ok(()),
            Command::MouseMove { .. } | Command::MouseButton { .. } | Command::Scroll { .. } | Command::Key { .. } | Command::Chord { .. } | Command::TypeText { .. } => {
                anyhow::ensure!(self.remote_input.load(std::sync::atomic::Ordering::Relaxed), "Remote keyboard and mouse is turned off on the computer");
                let op = match cmd {
                    Command::MouseMove { dx, dy } => {
                        let (dx, dy) = input::clamp_move(*dx, *dy);
                        input::Op::Move(dx, dy)
                    }
                    Command::MouseButton { button, action } => input::Op::Button(*button, *action),
                    Command::Scroll { dx, dy } => input::Op::Scroll((*dx).clamp(-50, 50), (*dy).clamp(-50, 50)),
                    Command::Key { key, ctrl, alt, shift, meta } => input::Op::Key { key: *key, ctrl: *ctrl, alt: *alt, shift: *shift, meta: *meta },
                    Command::Chord { ch, ctrl, alt, shift, meta } => input::Op::Chord { ch: *ch, ctrl: *ctrl, alt: *alt, shift: *shift, meta: *meta },
                    Command::TypeText { text } => {
                        input::check_text(text)?;
                        input::Op::Text(text.clone())
                    }
                    _ => unreachable!(),
                };
                blocking(move || input::send(op)).await?
            }
            Command::SetToggle { toggle, on } => {
                let (t, o) = (*toggle, *on);
                let r = blocking(move || toggles::set(t, o)).await?;
                // Read the new state on the next snapshot, not from the cache.
                *self.toggles.lock().unwrap() = None;
                r
            }
            Command::ClipboardSet { text } => {
                let (c, t) = (self.clipboard.clone(), text.clone());
                blocking(move || c.set(&t)).await?
            }
            Command::CloseWindow { window } => {
                let w = window.clone();
                blocking(move || windows::close(&w)).await?
            }
            Command::KillApp { window } => {
                let w = window.clone();
                blocking(move || windows::kill(&w)).await?
            }
            Command::SetWindowFullscreen { window, screen, on } => {
                let (w, s, on) = (window.clone(), screen.clone(), *on);
                blocking(move || windows::fullscreen(&w, s.as_deref(), on)).await?
            }
            Command::OpenStream { stream, screen } => {
                let (url, s) = (stream.url(), screen.clone());
                blocking(move || windows::open_url(url, s.as_deref())).await?
            }
            Command::TotalShutdown { delay, cut_after } => self.total.schedule(*delay, *cut_after).await,
            Command::CancelTotalShutdown => self.total.cancel().await,
            Command::Power { action } => {
                let a = *action;
                blocking(move || power::run(a)).await?
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchboard_core::protocol::RgbEffect;

    fn lit() -> System {
        System { rgb: rgb::Rgb::found(), ..Default::default() }
    }

    #[tokio::test]
    async fn rgb_commands_change_the_lighting() {
        let s = lit();
        for cmd in [
            Command::SetRgbPower { on: false },
            Command::SetRgbEffect { effect: RgbEffect::RainbowWave },
            // Levels past the end of the slider stop at 100.
            Command::SetRgbBrightness { level: 180 },
            Command::SetRgbSpeed { level: 101 },
            // Stored the one way the phone shows it: lower case, with the `#`.
            Command::SetRgbColor { color: "B30071".into() },
        ] {
            assert_eq!(s.run(&cmd).await.unwrap(), None, "{cmd:?}");
        }
        let now = s.rgb.state().unwrap();
        assert!(!now.on);
        assert_eq!(now.effect, RgbEffect::RainbowWave);
        assert_eq!((now.brightness, now.speed), (100, 100));
        assert_eq!(now.color, "#b30071");
    }

    #[tokio::test]
    async fn a_bad_colour_is_refused_and_changes_nothing() {
        let s = lit();
        let e = s.run(&Command::SetRgbColor { color: "#red".into() }).await.unwrap_err();
        assert!(e.to_string().contains("not a colour"), "{e}");
        assert_eq!(s.rgb.state().unwrap().color, "#ffffff");
        // Nothing was changed, so the lights are still left alone.
        assert!(!s.rgb.watch().borrow().managed);
    }

    #[tokio::test]
    async fn rgb_commands_fail_without_lighting() {
        let s = System::default();
        for cmd in [
            Command::SetRgbPower { on: true },
            Command::SetRgbBrightness { level: 10 },
            Command::SetRgbEffect { effect: RgbEffect::Static },
            Command::SetRgbColor { color: "#ffffff".into() },
            Command::SetRgbSpeed { level: 10 },
        ] {
            assert!(s.run(&cmd).await.is_err(), "{cmd:?}");
        }
    }

    #[tokio::test]
    async fn window_commands_need_a_real_window() {
        let s = System::default();
        for window in ["", "0", "not a number"] {
            for cmd in [
                Command::CloseWindow { window: window.into() },
                Command::KillApp { window: window.into() },
                Command::SetWindowFullscreen { window: window.into(), screen: None, on: true },
                Command::SetWindowFullscreen { window: window.into(), screen: None, on: false },
            ] {
                assert!(s.run(&cmd).await.is_err(), "{cmd:?}");
            }
        }
    }

    #[tokio::test]
    async fn a_stream_opens_only_on_a_screen_that_exists() {
        use switchboard_core::protocol::Stream;
        let s = System::default();
        // Refused before any browser is started, here and where it is not supported at all.
        assert!(s.run(&Command::OpenStream { stream: Stream::Netflix, screen: Some("no such screen".into()) }).await.is_err());
    }

    #[tokio::test]
    async fn total_shutdown_needs_a_plug() {
        let s = System::default();
        // Hidden from the phone, and refused if asked for anyway; the computer is not shut down.
        assert_eq!(s.total.state(), None);
        for delay in [0, 600] {
            let e = s.run(&Command::TotalShutdown { delay, cut_after: Some(600) }).await.unwrap_err();
            assert!(e.to_string().contains("No smart plug"), "{e}");
        }
        s.run(&Command::CancelTotalShutdown).await.unwrap();
    }

    #[tokio::test]
    async fn display_power_needs_a_known_display() {
        let s = System::default();
        let e = s.run(&Command::SetDisplayPower { display: "ddc:9".into(), on: false }).await.unwrap_err();
        assert!(e.to_string().contains("unknown display"), "{e}");
    }
}
