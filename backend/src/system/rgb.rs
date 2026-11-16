//! RGB lighting, through an OpenRGB server on this computer.
//!
//! OpenRGB talks to the hardware (board, memory, strips); this draws the
//! effects and sends it colours. Every device has its own list of built-in
//! modes, with different names and options, so those are not used: one small
//! set of effects is rendered here and sent LED by LED. That also gives a
//! brightness and an on/off that OpenRGB itself has no single switch for.
//!
//! OpenRGB knows the order of the LEDs in each zone (a strip, a fan header,
//! a memory stick) but not where the zones sit in the room, so an effect that
//! moves runs along every zone on its own rather than across the whole setup.
//!
//! Nothing is sent until the person changes a lighting control for the first
//! time. Installing Switchboard next to an OpenRGB setup leaves it alone.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use switchboard_core::protocol::{RgbEffect, RgbState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;

/// OpenRGB's SDK server; the port spells ORGB on a phone keypad.
const SERVER: (&str, u16) = ("127.0.0.1", 6742);
const RETRY: Duration = Duration::from_secs(5);
const FRAME: Duration = Duration::from_millis(33);
/// A steady colour is sent again this often, in case a device lost it.
const REFRESH: Duration = Duration::from_secs(2);
/// Devices come and go (a strip resized, a controller replugged).
const RESCAN: Duration = Duration::from_secs(30);

const REQUEST_CONTROLLER_COUNT: u32 = 0;
const REQUEST_CONTROLLER_DATA: u32 = 1;
const SET_CLIENT_NAME: u32 = 50;
const UPDATE_LEDS: u32 = 1050;
const SET_CUSTOM_MODE: u32 = 1100;

/// What is kept between runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    #[serde(flatten)]
    pub state: RgbState,
    /// Whether the person has used the lighting controls; until then the
    /// devices are left as they are.
    pub managed: bool,
}

impl Default for Saved {
    fn default() -> Self {
        Self { state: RgbState { on: true, brightness: 100, effect: RgbEffect::Static, color: "#ffffff".into(), speed: 50 }, managed: false }
    }
}

#[derive(Clone)]
pub struct Rgb {
    saved: Arc<watch::Sender<Saved>>,
    /// An OpenRGB server is up and has at least one LED.
    available: Arc<AtomicBool>,
}

impl Default for Rgb {
    fn default() -> Self {
        Self { saved: Arc::new(watch::channel(Saved::default()).0), available: Arc::default() }
    }
}

impl Rgb {
    /// Restore what `watch` last handed out to be stored.
    pub fn load(&self, bytes: &[u8]) {
        if let Ok(s) = serde_json::from_slice::<Saved>(bytes) {
            self.saved.send_replace(s);
        }
    }

    /// Follows the settings, for whoever stores them.
    pub fn watch(&self) -> watch::Receiver<Saved> {
        self.saved.subscribe()
    }

    pub fn state(&self) -> Option<RgbState> {
        self.available.load(Ordering::Relaxed).then(|| self.saved.borrow().state.clone())
    }

    pub fn change(&self, f: impl FnOnce(&mut RgbState)) -> Result<()> {
        anyhow::ensure!(self.available.load(Ordering::Relaxed), "No RGB lighting found. Is OpenRGB running with its SDK server on?");
        self.saved.send_modify(|s| {
            f(&mut s.state);
            s.managed = true;
        });
        Ok(())
    }

    /// Drives the lights for as long as the daemon lives, reconnecting when
    /// OpenRGB is closed and opened again.
    pub async fn run(self) {
        loop {
            if let Err(e) = self.session().await {
                tracing::debug!("openrgb: {e:#}");
            }
            self.available.store(false, Ordering::Relaxed);
            tokio::time::sleep(RETRY).await;
        }
    }

    async fn session(&self) -> Result<()> {
        let mut c = Client::connect().await?;
        let mut devs = c.controllers().await?;
        self.available.store(devs.iter().any(|d| d.leds() > 0), Ordering::Relaxed);
        tracing::info!("openrgb: {} controllers, {} LEDs", devs.len(), devs.iter().map(Device::leds).sum::<usize>());

        let mut rx = self.saved.subscribe();
        let mut render = Render::default();
        let mut tick = Instant::now();
        let mut scanned = Instant::now();
        let mut sent: Option<([u8; 3], Instant)> = None;
        // Direct mode is asked for once, the first time a colour goes out.
        let mut direct = false;
        loop {
            let s = rx.borrow_and_update().clone();
            let dt = tick.elapsed().as_secs_f32().min(0.25);
            tick = Instant::now();

            if scanned.elapsed() > RESCAN {
                let found = c.controllers().await?;
                if found != devs {
                    devs = found;
                    direct = false;
                    sent = None;
                }
                self.available.store(devs.iter().any(|d| d.leds() > 0), Ordering::Relaxed);
                scanned = Instant::now();
            }

            if s.managed {
                if s.state.on {
                    render.advance(&s.state, dt);
                }
                let paint = |pos: f32| if s.state.on { render.at(&s.state, pos) } else { [0; 3] };
                // The first LED stands for the frame: when it has not changed, nothing has.
                let color = paint(0.0);
                if sent.is_none_or(|(was, at)| was != color || at.elapsed() > REFRESH) {
                    for d in devs.iter().filter(|d| d.leds() > 0) {
                        if !direct {
                            c.send(d.index, SET_CUSTOM_MODE, &[]).await?;
                        }
                        c.paint(d, &paint).await?;
                    }
                    direct = true;
                    sent = Some((color, Instant::now()));
                }
            }

            let moving = s.managed && s.state.on && s.state.effect != RgbEffect::Static;
            if moving {
                tokio::time::sleep(FRAME).await;
            } else {
                tokio::select! {
                    _ = tokio::time::sleep(REFRESH) => {}
                    r = rx.changed() => r?,
                }
            }
        }
    }
}

#[cfg(test)]
impl Rgb {
    /// As if an OpenRGB server with LEDs had been found.
    pub(crate) fn found() -> Self {
        let rgb = Self::default();
        rgb.available.store(true, Ordering::Relaxed);
        rgb
    }
}

// --- Effects ----------------------------------------------------------------

/// One loop of SignalRGB's Neon Shift (pink, purple, blue, light blue) at its
/// default speed, as its renderer draws it: 164 frames, 50 a second.
///
/// The effect paints its target colour over the canvas at 5% opacity every
/// frame, and the 8-bit canvas rounds that so heavily that it never reaches
/// the colours the script names. Recomputing it from the script therefore
/// gives visibly different colours; these were recorded from the real thing.
const NEON_FPS: f32 = 50.0;
#[rustfmt::skip]
const NEON: [[u8; 3]; 164] = [
    [17, 100, 165], [25, 95, 163], [34, 89, 161], [41, 84, 159], [47, 80, 157], [54, 77, 155], [60, 73, 152], [67, 69, 150],
    [73, 65, 148], [77, 62, 146], [82, 58, 145], [87, 55, 144], [93, 52, 143], [97, 49, 142], [101, 46, 141], [105, 45, 140],
    [108, 43, 139], [111, 41, 139], [114, 39, 138], [117, 37, 137], [120, 35, 136], [124, 33, 135], [127, 31, 134], [130, 28, 133],
    [132, 27, 132], [134, 26, 131], [136, 25, 130], [138, 24, 129], [139, 23, 128], [141, 22, 128], [143, 21, 128], [145, 20, 128],
    [147, 19, 128], [148, 18, 128], [149, 17, 128], [150, 16, 128], [151, 15, 128], [152, 15, 128], [153, 14, 128], [155, 13, 128],
    [156, 12, 128], [151, 11, 129], [147, 10, 129], [143, 9, 129], [140, 9, 129], [138, 9, 129], [135, 9, 129], [132, 9, 129],
    [129, 9, 129], [126, 9, 129], [124, 9, 129], [121, 9, 129], [119, 9, 129], [117, 9, 129], [115, 9, 129], [113, 9, 129],
    [111, 9, 129], [109, 9, 129], [108, 9, 129], [107, 9, 129], [106, 9, 129], [105, 9, 129], [104, 9, 129], [103, 9, 129],
    [102, 9, 129], [101, 9, 129], [100, 9, 129], [99, 9, 129], [98, 9, 129], [97, 9, 129], [96, 9, 129], [95, 9, 129],
    [94, 9, 129], [93, 9, 129], [91, 9, 129], [90, 9, 129], [89, 9, 129], [88, 9, 129], [87, 9, 129], [87, 9, 129],
    [87, 9, 129], [87, 9, 129], [83, 10, 129], [79, 10, 129], [76, 10, 129], [72, 10, 129], [68, 10, 129], [65, 10, 129],
    [62, 10, 129], [58, 10, 129], [55, 10, 129], [52, 10, 129], [49, 10, 129], [46, 10, 129], [45, 10, 129], [43, 10, 129],
    [41, 10, 129], [39, 10, 129], [37, 10, 129], [35, 10, 129], [33, 10, 129], [31, 10, 129], [28, 10, 129], [27, 10, 129],
    [26, 10, 129], [25, 10, 129], [24, 10, 129], [23, 10, 129], [22, 10, 129], [21, 10, 129], [20, 10, 129], [19, 10, 129],
    [18, 10, 129], [17, 10, 129], [16, 10, 129], [15, 10, 129], [15, 10, 129], [14, 10, 129], [13, 10, 129], [12, 10, 129],
    [11, 10, 129], [10, 10, 129], [9, 10, 129], [9, 15, 131], [9, 19, 133], [9, 24, 135], [9, 29, 137], [9, 34, 139],
    [9, 38, 140], [9, 42, 142], [9, 46, 144], [9, 49, 146], [9, 52, 148], [9, 55, 149], [9, 58, 150], [9, 62, 151],
    [9, 65, 152], [9, 68, 153], [9, 71, 155], [9, 73, 156], [9, 75, 157], [9, 77, 158], [9, 78, 159], [9, 80, 160],
    [9, 82, 161], [9, 84, 162], [9, 86, 163], [9, 88, 164], [9, 89, 165], [9, 90, 166], [9, 91, 167], [9, 93, 168],
    [9, 94, 168], [9, 95, 168], [9, 96, 168], [9, 97, 168], [9, 98, 168], [9, 99, 168], [9, 100, 168], [9, 101, 168],
    [9, 102, 168], [9, 103, 168], [9, 104, 168], [9, 105, 168],
];
const BREATH: f32 = 4.0;
const RAINBOW: f32 = 10.0;

/// Where an effect is in its loop.
#[derive(Default)]
pub struct Render {
    /// Seconds of effect time; runs faster or slower than the clock with `speed`.
    phase: f32,
}

impl Render {
    /// Move on by `dt` seconds.
    pub fn advance(&mut self, s: &RgbState, dt: f32) {
        // Speed 50 is the effect's natural pace; 100 is twice that.
        self.phase += dt * f32::from(s.speed.clamp(1, 100)) / 50.0;
    }

    /// `advance`, then the colour of an effect that is the same everywhere.
    #[cfg(test)]
    pub fn frame(&mut self, s: &RgbState, dt: f32) -> [u8; 3] {
        self.advance(s, dt);
        self.at(s, 0.0)
    }

    /// The colour now of the LED this far along its zone, from 0 at the first
    /// to just under 1 at the last. Only the effects that move use `pos`.
    pub fn at(&self, s: &RgbState, pos: f32) -> [u8; 3] {
        let base = parse_color(&s.color).unwrap_or([255; 3]).map(f32::from);
        let c = match s.effect {
            RgbEffect::Static => base,
            RgbEffect::Breathing => {
                let k = 0.5 - 0.5 * (std::f32::consts::TAU * self.phase / BREATH).cos();
                base.map(|v| v * k)
            }
            RgbEffect::Rainbow => hue(self.phase / RAINBOW * 360.0),
            // One whole wheel fits on the zone and slides towards its far end.
            RgbEffect::RainbowWave => hue((self.phase / RAINBOW - pos) * 360.0),
            RgbEffect::NeonShift => {
                // On a recorded frame at the recorded pace; between frames at other speeds.
                let at = self.phase * NEON_FPS;
                let (a, b) = (NEON[at as usize % NEON.len()], NEON[(at as usize + 1) % NEON.len()]);
                let k = at.fract();
                std::array::from_fn(|i| f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * k)
            }
        };
        let b = f32::from(s.brightness.min(100)) / 100.0;
        c.map(|v| (v * b).round().clamp(0.0, 255.0) as u8)
    }
}

/// A fully saturated colour at this angle on the colour wheel.
fn hue(deg: f32) -> [f32; 3] {
    let h = deg.rem_euclid(360.0) / 60.0;
    let x = 255.0 * (1.0 - (h % 2.0 - 1.0).abs());
    match h as u8 {
        0 => [255.0, x, 0.0],
        1 => [x, 255.0, 0.0],
        2 => [0.0, 255.0, x],
        3 => [0.0, x, 255.0],
        4 => [x, 0.0, 255.0],
        _ => [255.0, 0.0, x],
    }
}

/// `#rrggbb`, with or without the `#`.
pub fn parse_color(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#').unwrap_or(s);
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

// --- OpenRGB SDK client -----------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Device {
    index: u32,
    /// LEDs in each zone, in the order the device numbers them.
    zones: Vec<usize>,
}

impl Device {
    fn leds(&self) -> usize {
        self.zones.iter().sum()
    }
}

/// The few packets needed, in the first version of the protocol: a client
/// that asks for nothing newer is answered in it by every server.
struct Client {
    s: TcpStream,
}

impl Client {
    async fn connect() -> Result<Self> {
        let s = tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(SERVER)).await.map_err(|_| anyhow!("no answer"))??;
        s.set_nodelay(true)?;
        let mut c = Self { s };
        c.send(0, SET_CLIENT_NAME, b"Switchboard\0").await?;
        Ok(c)
    }

    async fn send(&mut self, device: u32, id: u32, body: &[u8]) -> Result<()> {
        let mut p = Vec::with_capacity(16 + body.len());
        p.extend_from_slice(b"ORGB");
        for v in [device, id, body.len() as u32] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        p.extend_from_slice(body);
        self.s.write_all(&p).await?;
        Ok(())
    }

    /// The body of the next packet with this id. The server also sends
    /// notices nobody asked for (the device list changed); those are skipped.
    async fn reply(&mut self, want: u32) -> Result<Vec<u8>> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut h = [0u8; 16];
                self.s.read_exact(&mut h).await?;
                if &h[..4] != b"ORGB" {
                    bail!("not an OpenRGB server");
                }
                let id = u32::from_le_bytes(h[8..12].try_into().unwrap());
                let len = u32::from_le_bytes(h[12..16].try_into().unwrap()) as usize;
                anyhow::ensure!(len <= 16 << 20, "oversized packet");
                let mut body = vec![0u8; len];
                self.s.read_exact(&mut body).await?;
                if id == want {
                    return Ok(body);
                }
            }
        })
        .await
        .map_err(|_| anyhow!("the server stopped answering"))?
    }

    async fn controllers(&mut self) -> Result<Vec<Device>> {
        self.send(0, REQUEST_CONTROLLER_COUNT, &[]).await?;
        let n = self.reply(REQUEST_CONTROLLER_COUNT).await?;
        let n = u32::from_le_bytes(n.get(..4).ok_or_else(|| anyhow!("short reply"))?.try_into().unwrap());
        let mut out = Vec::new();
        for index in 0..n.min(256) {
            self.send(index, REQUEST_CONTROLLER_DATA, &0u32.to_le_bytes()).await?;
            let data = self.reply(REQUEST_CONTROLLER_DATA).await?;
            out.push(Device { index, zones: zones(&data).unwrap_or_default() });
        }
        Ok(out)
    }

    /// Colours every LED of a device; `color` is asked how far along its zone each one is.
    async fn paint(&mut self, d: &Device, color: &impl Fn(f32) -> [u8; 3]) -> Result<()> {
        let leds = d.leds();
        let mut body = Vec::with_capacity(6 + leds * 4);
        body.extend_from_slice(&((6 + leds * 4) as u32).to_le_bytes());
        body.extend_from_slice(&(leds as u16).to_le_bytes());
        for &n in &d.zones {
            for i in 0..n {
                let c = color(i as f32 / n as f32);
                body.extend_from_slice(&[c[0], c[1], c[2], 0]);
            }
        }
        self.send(d.index, UPDATE_LEDS, &body).await
    }
}

/// How many LEDs each zone of a controller has, from its description
/// (protocol version 0). The zones sit after the modes, which have to be
/// walked over. A controller whose zones do not add up to its LED count is
/// treated as one zone.
fn zones(data: &[u8]) -> Option<Vec<usize>> {
    struct R<'a>(&'a [u8]);
    impl R<'_> {
        fn skip(&mut self, n: usize) -> Option<()> {
            self.0 = self.0.get(n..)?;
            Some(())
        }
        fn u16(&mut self) -> Option<usize> {
            let v = u16::from_le_bytes(self.0.get(..2)?.try_into().ok()?);
            self.skip(2)?;
            Some(v as usize)
        }
        fn u32(&mut self) -> Option<usize> {
            let v = u32::from_le_bytes(self.0.get(..4)?.try_into().ok()?);
            self.skip(4)?;
            Some(v as usize)
        }
        fn string(&mut self) -> Option<()> {
            let n = self.u16()?;
            self.skip(n)
        }
    }
    let mut r = R(data);
    r.skip(8)?; // size, type
    for _ in 0..5 {
        r.string()?; // name, description, version, serial, location
    }
    let modes = r.u16()?;
    r.skip(4)?; // active mode
    for _ in 0..modes {
        r.string()?;
        r.skip(36)?; // value, flags, speed and colour ranges, speed, direction, colour mode
        let colors = r.u16()?;
        r.skip(colors * 4)?;
    }
    let count = r.u16()?;
    let mut zones = Vec::with_capacity(count);
    for _ in 0..count {
        r.string()?;
        r.skip(12)?; // type, LED minimum and maximum
        zones.push(r.u32()?);
        let matrix = r.u16()?;
        r.skip(matrix)?;
    }
    let leds = r.u16()?;
    Some(if zones.iter().sum::<usize>() == leds { zones } else { vec![leds] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(effect: RgbEffect) -> RgbState {
        RgbState { on: true, brightness: 100, effect, color: "#ff8000".into(), speed: 50 }
    }

    #[test]
    fn colors_round_trip() {
        assert_eq!(parse_color("#B30071"), Some([179, 0, 113]));
        assert_eq!(parse_color("0071b3"), Some([0, 113, 179]));
        assert_eq!(hex([179, 0, 113]), "#b30071");
        for bad in ["", "#fff", "#gggggg", "#ff80000", "#ééé"] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
    }

    #[test]
    fn static_scales_with_brightness() {
        let mut r = Render::default();
        assert_eq!(r.frame(&state(RgbEffect::Static), 0.05), [255, 128, 0]);
        let dim = RgbState { brightness: 50, ..state(RgbEffect::Static) };
        assert_eq!(r.frame(&dim, 0.05), [128, 64, 0]);
        let dark = RgbState { brightness: 0, ..state(RgbEffect::Static) };
        assert_eq!(r.frame(&dark, 0.05), [0, 0, 0]);
    }

    #[test]
    fn breathing_goes_dark_and_back() {
        let mut r = Render::default();
        let s = state(RgbEffect::Breathing);
        assert_eq!(r.frame(&s, 0.0), [0, 0, 0]);
        // Half a breath later it is at full colour.
        assert_eq!(r.frame(&s, BREATH / 2.0), [255, 128, 0]);
    }

    #[test]
    fn rainbow_walks_the_wheel() {
        let mut r = Render::default();
        let s = state(RgbEffect::Rainbow);
        assert_eq!(r.frame(&s, 0.0), [255, 0, 0]);
        assert_eq!(r.frame(&s, RAINBOW / 3.0), [0, 255, 0]);
        assert_eq!(r.frame(&s, RAINBOW / 3.0), [0, 0, 255]);
    }

    #[test]
    fn neon_plays_the_recorded_loop() {
        let s = state(RgbEffect::NeonShift);
        let frame = 1.0 / NEON_FPS;
        // At the default speed every recorded frame comes out as recorded, and the loop closes.
        let mut r = Render::default();
        assert_eq!(r.frame(&s, 0.0), NEON[0]);
        r.phase = 41.0 * frame;
        assert_eq!(r.frame(&s, 0.0), NEON[41]);
        r.phase = 164.0 * frame;
        assert_eq!(r.frame(&s, 0.0), NEON[0]);
        // Half the speed lands halfway between two frames.
        let slow = RgbState { speed: 25, ..s };
        let mut r = Render::default();
        let c = r.frame(&slow, frame);
        for i in 0..3 {
            let (lo, hi) = (NEON[0][i].min(NEON[1][i]), NEON[0][i].max(NEON[1][i]));
            assert!((lo..=hi).contains(&c[i]), "{c:?}");
        }
    }

    #[test]
    fn speed_changes_the_pace() {
        let fast = RgbState { speed: 100, ..state(RgbEffect::Rainbow) };
        let mut r = Render::default();
        // Twice the speed covers a third of the wheel in half the time.
        assert_eq!(r.frame(&fast, RAINBOW / 6.0), [0, 255, 0]);
    }

    #[test]
    fn saved_settings_round_trip() {
        let s = Saved { state: state(RgbEffect::NeonShift), managed: true };
        let rgb = Rgb::default();
        rgb.load(&serde_json::to_vec(&s).unwrap());
        assert_eq!(*rgb.watch().borrow(), s);
        // Garbage keeps the defaults.
        let rgb = Rgb::default();
        rgb.load(b"not json");
        assert_eq!(*rgb.watch().borrow(), Saved::default());
    }

    #[test]
    fn hidden_and_refused_without_a_server() {
        let rgb = Rgb::default();
        assert_eq!(rgb.state(), None);
        assert!(rgb.change(|s| s.on = false).is_err());
        assert!(!rgb.watch().borrow().managed);
    }

    #[test]
    fn wave_spreads_the_wheel_along_a_zone() {
        let s = state(RgbEffect::RainbowWave);
        let r = Render::default();
        // A third of the way along is a third of the wheel behind.
        assert_eq!(r.at(&s, 0.0), [255, 0, 0]);
        assert_eq!(r.at(&s, 1.0 / 3.0), [0, 0, 255]);
        assert_eq!(r.at(&s, 2.0 / 3.0), [0, 255, 0]);
        // And it moves: later, a LED shows what the one before it showed.
        let mut r = Render::default();
        r.advance(&s, RAINBOW / 3.0);
        assert_eq!(r.at(&s, 1.0 / 3.0), [255, 0, 0]);
        // The effects that are one colour everywhere ignore the position.
        let s = state(RgbEffect::NeonShift);
        assert_eq!(r.at(&s, 0.0), r.at(&s, 0.7));
    }

    /// A controller description the way OpenRGB lays it out: a 5-LED zone
    /// and one of `strip` LEDs, and `leds` as the total it reports.
    fn controller(strip: u32, leds: u16) -> Vec<u8> {
        fn s(out: &mut Vec<u8>, v: &str) {
            out.extend_from_slice(&(v.len() as u16 + 1).to_le_bytes());
            out.extend_from_slice(v.as_bytes());
            out.push(0);
        }
        let mut d = vec![0u8; 8];
        for v in ["Board", "desc", "1.0", "serial", "HID: x"] {
            s(&mut d, v);
        }
        d.extend_from_slice(&2u16.to_le_bytes()); // modes
        d.extend_from_slice(&0u32.to_le_bytes());
        for (name, colors) in [("Direct", 0u16), ("Static", 2)] {
            s(&mut d, name);
            d.extend_from_slice(&[0u8; 36]);
            d.extend_from_slice(&colors.to_le_bytes());
            d.extend_from_slice(&vec![0u8; colors as usize * 4]);
        }
        d.extend_from_slice(&2u16.to_le_bytes()); // zones
        for (name, count) in [("Board", 5u32), ("Strip", strip)] {
            s(&mut d, name);
            d.extend_from_slice(&[0u8; 12]);
            d.extend_from_slice(&count.to_le_bytes());
            d.extend_from_slice(&3u16.to_le_bytes());
            d.extend_from_slice(&[0u8; 3]);
        }
        d.extend_from_slice(&leds.to_le_bytes());
        d
    }

    #[test]
    fn reads_zones() {
        assert_eq!(zones(&controller(120, 125)), Some(vec![5, 120]));
        assert_eq!(zones(&controller(0, 5)), Some(vec![5, 0]));
        // Zones that do not add up fall back to the total.
        assert_eq!(zones(&controller(120, 7)), Some(vec![7]));
        let whole = controller(1, 6);
        assert_eq!(zones(&whole[..whole.len() - 1]), None);
        assert_eq!(zones(&[]), None);
    }

    #[test]
    fn a_change_is_kept_and_takes_over_the_lights() {
        let rgb = Rgb::found();
        // Found, but untouched: shown to the phone, not yet driven.
        assert_eq!(rgb.state(), Some(Saved::default().state));
        assert!(!rgb.watch().borrow().managed);
        rgb.change(|s| s.brightness = 40).unwrap();
        assert_eq!(rgb.state().unwrap().brightness, 40);
        assert!(rgb.watch().borrow().managed);
    }

    #[test]
    fn off_and_out_of_range_values_stay_in_bounds() {
        let r = Render::default();
        // Brightness and speed beyond 100 count as 100, and a colour that cannot be read is white.
        let loud = RgbState { brightness: 250, ..state(RgbEffect::Static) };
        assert_eq!(r.at(&loud, 0.0), [255, 128, 0]);
        let odd = RgbState { color: "nope".into(), ..state(RgbEffect::Static) };
        assert_eq!(r.at(&odd, 0.0), [255, 255, 255]);
        let mut fast = Render::default();
        fast.advance(&RgbState { speed: 255, ..state(RgbEffect::Rainbow) }, 1.0);
        assert_eq!(fast.phase, 2.0);
        // Speed 0 still moves, slowly, rather than freezing the effect.
        let mut slow = Render::default();
        slow.advance(&RgbState { speed: 0, ..state(RgbEffect::Rainbow) }, 1.0);
        assert!(slow.phase > 0.0);
    }

    /// A client joined to this end of a socket, standing in for the OpenRGB server.
    async fn server() -> (Client, TcpStream) {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let s = TcpStream::connect(l.local_addr().unwrap()).await.unwrap();
        let (server, _) = l.accept().await.unwrap();
        (Client { s }, server)
    }

    /// The next packet the client sent: device, id, body.
    async fn packet(s: &mut TcpStream) -> (u32, u32, Vec<u8>) {
        let mut h = [0u8; 16];
        s.read_exact(&mut h).await.unwrap();
        assert_eq!(&h[..4], b"ORGB");
        let n = |i: usize| u32::from_le_bytes(h[i..i + 4].try_into().unwrap());
        let mut body = vec![0u8; n(12) as usize];
        s.read_exact(&mut body).await.unwrap();
        (n(4), n(8), body)
    }

    async fn answer(s: &mut TcpStream, device: u32, id: u32, body: &[u8]) {
        let mut p = b"ORGB".to_vec();
        for v in [device, id, body.len() as u32] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        p.extend_from_slice(body);
        s.write_all(&p).await.unwrap();
    }

    #[tokio::test]
    async fn paints_every_led_by_its_place_in_its_zone() {
        let (mut c, mut s) = server().await;
        let d = Device { index: 3, zones: vec![2, 1] };
        c.paint(&d, &|pos| [(pos * 100.0) as u8, 1, 2]).await.unwrap();
        let (device, id, body) = packet(&mut s).await;
        assert_eq!((device, id), (3, UPDATE_LEDS));
        let mut want = 18u32.to_le_bytes().to_vec();
        want.extend_from_slice(&3u16.to_le_bytes());
        // Two LEDs at 0 and halfway along the first zone, then the second zone starts again at 0.
        want.extend_from_slice(&[0, 1, 2, 0, 50, 1, 2, 0, 0, 1, 2, 0]);
        assert_eq!(body, want);
    }

    #[tokio::test]
    async fn lists_controllers_past_unasked_notices() {
        let (mut c, mut s) = server().await;
        let fake = async {
            assert_eq!(packet(&mut s).await, (0, REQUEST_CONTROLLER_COUNT, vec![]));
            // "The device list changed", which nobody asked for, comes first.
            answer(&mut s, 0, 100, &[]).await;
            answer(&mut s, 0, REQUEST_CONTROLLER_COUNT, &3u32.to_le_bytes()).await;
            for (index, data) in [controller(120, 125), controller(120, 7), vec![1, 2, 3]].into_iter().enumerate() {
                // Each is asked for in protocol version 0.
                assert_eq!(packet(&mut s).await, (index as u32, REQUEST_CONTROLLER_DATA, vec![0; 4]));
                answer(&mut s, index as u32, REQUEST_CONTROLLER_DATA, &data).await;
            }
        };
        let (found, ()) = tokio::join!(c.controllers(), fake);
        let found = found.unwrap();
        assert_eq!(
            found,
            vec![
                Device { index: 0, zones: vec![5, 120] },
                Device { index: 1, zones: vec![7] },
                // A description that cannot be read is a device with no LEDs, not an error.
                Device { index: 2, zones: vec![] },
            ]
        );
        assert_eq!(found[2].leds(), 0);
    }

    #[tokio::test]
    async fn refuses_a_server_that_is_not_openrgb() {
        let (mut c, mut s) = server().await;
        s.write_all(b"HTTP/1.1 400 Bad").await.unwrap();
        assert!(c.reply(REQUEST_CONTROLLER_COUNT).await.is_err());
    }
}
