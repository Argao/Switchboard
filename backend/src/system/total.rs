//! Total shutdown: the computer shuts down, and then the smart plug it is
//! plugged into cuts the power.
//!
//! Once the computer is off nothing here runs, so the cut is not something
//! this can do afterwards. The plug is told beforehand to switch off by
//! itself after a delay, and counts it alone.
//!
//! That makes the delay a bet that the shutdown finishes inside it. Losing
//! the bet cuts the power on a running computer, so:
//!
//! - an update waiting to install on shutdown, which can take many minutes,
//!   refuses the whole thing;
//! - the plug has to confirm the delay before the computer is told to shut
//!   down, and a plug that does not answer means the computer stays on;
//! - the shutdown is a forced one. A program asking whether to save can hold
//!   an ordinary shutdown up for as long as nobody answers, and this daemon
//!   is among the first things closed, so it would not be there to call the
//!   plug off. Unsaved work is lost either way once the power goes;
//! - while this is still running some way into the delay, the shutdown has
//!   stalled all the same, and the plug is told to stay on.
//!
//! Past the point where this is ended by the shutdown, only the length of
//! the delay protects the rest of it.

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use switchboard_core::protocol::TotalShutdown;
use tokio::sync::{Notify, watch};

use super::tuya::Plug;

/// How long the plug waits, from the shutdown, unless set otherwise.
pub const CUT_AFTER: u32 = 300;
/// A shutdown takes most of a minute on a slow disk; a plug's own timer goes to a day.
const CUT_RANGE: std::ops::RangeInclusive<u32> = 60..=1800;
/// Furthest ahead a total shutdown can be scheduled: a day.
pub const MAX_DELAY: u32 = 86_400;
/// A schedule found this far past its time (the computer was asleep, or off)
/// is dropped, not run late: shutting down under someone who has just woken
/// the computer is worse than not shutting down.
const LATE: u64 = 120;

/// What is kept between runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    /// When the shutdown starts, in Unix seconds, so it survives a restart and a sleep.
    pub at: Option<u64>,
    pub cut_after: u32,
}

impl Default for Saved {
    fn default() -> Self {
        Self { at: None, cut_after: CUT_AFTER }
    }
}

type Hook<T> = Box<dyn Fn() -> T + Send + Sync>;

pub struct Total {
    plug: Mutex<Option<Plug>>,
    saved: watch::Sender<Saved>,
    /// When the plug cuts the power, once the shutdown has been sent.
    cut_at: Mutex<Option<u64>>,
    /// Why the last one did not happen, for a phone that was not there to be told.
    error: Mutex<Option<String>>,
    wake: Notify,
    /// Held while one is being started, called off or checked on. Two phones
    /// asking at once would otherwise both shut the computer down, and the
    /// second, refused, would disarm the plug the first had armed.
    gate: tokio::sync::Mutex<()>,
    shut_down: Hook<Result<()>>,
    pending_update: Hook<bool>,
}

impl Default for Total {
    fn default() -> Self {
        Self::with(Box::new(super::power::shut_down_forced), Box::new(pending_update))
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// How long before the cut a computer that is still up means the shutdown stalled.
fn guard(cut_after: u32) -> u64 {
    u64::from((cut_after / 5).max(20))
}

const NO_PLUG: &str = "No smart plug is set up on the computer";
const UPDATE: &str = "An update is waiting to install when this computer shuts down, which can take longer than the plug waits. Shut down normally this time.";

impl Total {
    fn with(shut_down: Hook<Result<()>>, pending_update: Hook<bool>) -> Self {
        Self { plug: Mutex::default(), saved: watch::channel(Saved::default()).0, cut_at: Mutex::default(), error: Mutex::default(), wake: Notify::new(), gate: tokio::sync::Mutex::new(()), shut_down, pending_update }
    }

    pub fn set_plug(&self, plug: Option<Plug>) {
        let gone = plug.is_none();
        *self.plug.lock().unwrap() = plug;
        if gone {
            // Nothing can be cut without it.
            self.saved.send_if_modified(|s| s.at.take().is_some());
        }
    }

    pub fn plug(&self) -> Option<Plug> {
        self.plug.lock().unwrap().clone()
    }

    /// Restore what `watch` last handed out to be stored.
    pub fn load(&self, bytes: &[u8]) {
        if let Ok(mut s) = serde_json::from_slice::<Saved>(bytes) {
            s.cut_after = s.cut_after.clamp(*CUT_RANGE.start(), *CUT_RANGE.end());
            self.saved.send_replace(s);
        }
    }

    /// Follows the schedule and settings, for whoever stores them.
    pub fn watch(&self) -> watch::Receiver<Saved> {
        self.saved.subscribe()
    }

    pub fn set_cut_after(&self, secs: u32) {
        let secs = secs.clamp(*CUT_RANGE.start(), *CUT_RANGE.end());
        self.saved.send_if_modified(|s| std::mem::replace(&mut s.cut_after, secs) != secs);
    }

    pub fn state(&self) -> Option<TotalShutdown> {
        self.plug.lock().unwrap().as_ref()?;
        let saved = self.saved.borrow().clone();
        Some(TotalShutdown { at: saved.at, cut_after: saved.cut_after, cut_at: *self.cut_at.lock().unwrap(), pending_update: (self.pending_update)(), error: self.error.lock().unwrap().clone() })
    }

    /// Start one `delay` seconds from now; zero is at once. `cut_after`,
    /// when given, is how long the plug waits, from this one on.
    pub async fn schedule(&self, delay: u32, cut_after: Option<u32>) -> Result<()> {
        self.schedule_with(delay, cut_after, now()).await
    }

    #[cfg(test)]
    async fn schedule_at(&self, delay: u32, now: u64) -> Result<()> {
        self.schedule_with(delay, None, now).await
    }

    async fn schedule_with(&self, delay: u32, cut_after: Option<u32>, now: u64) -> Result<()> {
        let _one = self.gate.lock().await;
        let plug = self.plug().ok_or_else(|| anyhow!(NO_PLUG))?;
        ensure!(delay <= MAX_DELAY, "A total shutdown can be set up to a day ahead");
        ensure!(self.cut_at.lock().unwrap().is_none(), "This computer is already shutting down");
        ensure!(!(self.pending_update)(), UPDATE);
        // The wait has to be in place before the plug is armed with it, and
        // goes back to what it was when the request is refused after all: a
        // refused request changes nothing.
        let before = self.saved.borrow().cut_after;
        if let Some(secs) = cut_after {
            self.set_cut_after(secs);
        }
        let started = self.start(delay, &plug, now).await;
        if started.is_err() {
            self.set_cut_after(before);
        }
        started
    }

    async fn start(&self, delay: u32, plug: &Plug, now: u64) -> Result<()> {
        if delay == 0 {
            return self.fire(now).await;
        }
        // Find out now, while someone is there to be told, not at the time set.
        let state = plug.state().await?;
        ensure!(state.on, "The plug says it is switched off, so this computer is not plugged into it");
        *self.error.lock().unwrap() = None;
        self.saved.send_modify(|s| s.at = Some(now + u64::from(delay)));
        self.wake.notify_one();
        Ok(())
    }

    /// Call off one that is scheduled, or tell the plug to stay on when the
    /// shutdown has already been sent. The shutdown itself cannot be taken back.
    pub async fn cancel(&self) -> Result<()> {
        let _one = self.gate.lock().await;
        self.saved.send_if_modified(|s| s.at.take().is_some());
        *self.error.lock().unwrap() = None;
        if self.cut_at.lock().unwrap().is_some() {
            self.keep_on().await?;
            *self.cut_at.lock().unwrap() = None;
        }
        Ok(())
    }

    /// Arm the plug, then shut down. In that order: a plug that cannot be
    /// armed must leave the computer running, not off with the power still on.
    async fn fire(&self, now: u64) -> Result<()> {
        self.saved.send_if_modified(|s| s.at.take().is_some());
        let plug = self.plug().ok_or_else(|| anyhow!(NO_PLUG))?;
        ensure!(!(self.pending_update)(), UPDATE);
        let cut_after = self.saved.borrow().cut_after;
        plug.set_countdown(cut_after).await?;
        *self.cut_at.lock().unwrap() = Some(now + u64::from(cut_after));
        if let Err(e) = (self.shut_down)() {
            let kept = self.keep_on().await;
            *self.cut_at.lock().unwrap() = None;
            kept?;
            return Err(e.context("This computer would not shut down, so the plug was left on"));
        }
        *self.error.lock().unwrap() = None;
        self.wake.notify_one();
        Ok(())
    }

    /// Takes the plug's timer back. Worth more than one try: the alternative is losing power.
    async fn keep_on(&self) -> Result<()> {
        let plug = self.plug().ok_or_else(|| anyhow!(NO_PLUG))?;
        let mut last = None;
        for _ in 0..3 {
            match plug.set_countdown(0).await {
                Ok(()) => return Ok(()),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap().context("The plug could not be told to stay on and will still cut the power; switch it from its own app"))
    }

    /// Whatever is due at `now`: a scheduled shutdown whose time has come, or
    /// a shutdown that should have ended this long ago.
    async fn tick(&self, now: u64) {
        let _one = self.gate.lock().await;
        let at = self.saved.borrow().at;
        if let Some(at) = at.filter(|&at| now >= at) {
            if now - at > LATE {
                self.saved.send_if_modified(|s| s.at.take().is_some());
                *self.error.lock().unwrap() = Some("This computer was asleep or off at the time set, so the total shutdown was skipped".into());
            } else if let Err(e) = self.fire(now).await {
                *self.error.lock().unwrap() = Some(format!("{e:#}"));
            }
        }
        let cut_at = *self.cut_at.lock().unwrap();
        let cut_after = self.saved.borrow().cut_after;
        if cut_at.is_some_and(|cut| now + guard(cut_after) >= cut) {
            let note = match self.keep_on().await {
                Ok(()) => "This computer did not shut down in time, so the plug was left on".to_owned(),
                Err(e) => format!("This computer did not shut down in time. {e:#}"),
            };
            *self.cut_at.lock().unwrap() = None;
            *self.error.lock().unwrap() = Some(note);
        }
    }

    /// Runs what is scheduled and watches a shutdown under way, for as long
    /// as the daemon lives. Idle when there is neither.
    pub async fn run(self: std::sync::Arc<Self>) {
        loop {
            let busy = self.saved.borrow().at.is_some() || self.cut_at.lock().unwrap().is_some();
            if busy {
                // By the clock, not by counting sleeps: a computer that slept wakes up late.
                tokio::time::sleep(Duration::from_secs(1)).await;
                self.tick(now()).await;
            } else {
                self.wake.notified().await;
            }
        }
    }
}

/// Whether Windows has an update that installs on the next shutdown or
/// restart. It marks that with a registry key that exists only until then.
#[cfg(windows)]
pub fn pending_update() -> bool {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW};
    use windows::core::w;
    [
        w!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired"),
        w!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending"),
    ]
    .into_iter()
    .any(|path| {
        let mut key = HKEY::default();
        // SAFETY: `key` outlives the call, and is closed when it was opened.
        unsafe {
            let found = RegOpenKeyExW(HKEY_LOCAL_MACHINE, path, 0, KEY_READ, &mut key) == ERROR_SUCCESS;
            if found {
                let _ = RegCloseKey(key);
            }
            found
        }
    })
}

// shortcut: elsewhere updates are not installed during shutdown the way Windows does it.
#[cfg(not(windows))]
pub fn pending_update() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::super::tuya::tests::{Device, plug};
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const NOW: u64 = 1_800_000_000;

    struct Rig {
        total: Total,
        dev: Arc<Mutex<Device>>,
        /// Times the computer was told to shut down.
        shutdowns: Arc<AtomicUsize>,
        update: Arc<AtomicBool>,
        refuse: Arc<AtomicBool>,
    }

    /// A total shutdown wired to a made-up plug and a computer that only counts.
    async fn rig(dev: Device) -> Rig {
        let (p, dev) = plug(dev).await;
        let (shutdowns, update, refuse) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (n, r, u) = (shutdowns.clone(), refuse.clone(), update.clone());
        let total = Total::with(
            Box::new(move || {
                ensure!(!r.load(Ordering::Relaxed), "shutdown failed");
                n.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }),
            Box::new(move || u.load(Ordering::Relaxed)),
        );
        total.set_plug(Some(p));
        Rig { total, dev, shutdowns, update, refuse }
    }

    fn on() -> Device {
        Device { on: true, ..Default::default() }
    }

    #[tokio::test]
    async fn hidden_and_refused_without_a_plug() {
        let t = Total::with(Box::new(|| Ok(())), Box::new(|| false));
        assert_eq!(t.state(), None);
        assert_eq!(t.schedule_at(0, NOW).await.unwrap_err().to_string(), NO_PLUG);
        assert_eq!(t.schedule_at(60, NOW).await.unwrap_err().to_string(), NO_PLUG);
        // Nothing to call off is not an error.
        t.cancel().await.unwrap();
    }

    #[tokio::test]
    async fn arms_the_plug_then_shuts_down() {
        let r = rig(on()).await;
        r.total.schedule_at(0, NOW).await.unwrap();
        assert_eq!(r.dev.lock().unwrap().countdown, CUT_AFTER);
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 1);
        let s = r.total.state().unwrap();
        assert_eq!((s.at, s.cut_at, s.error), (None, Some(NOW + u64::from(CUT_AFTER)), None));
        // One at a time.
        assert!(r.total.schedule_at(0, NOW).await.is_err());
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn two_at_once_shut_down_once_and_leave_the_plug_armed() {
        let r = rig(on()).await;
        let (a, b) = tokio::join!(r.total.schedule_at(0, NOW), r.total.schedule_at(0, NOW));
        assert!(a.is_ok() != b.is_ok(), "{a:?} {b:?}");
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 1);
        assert_eq!(r.dev.lock().unwrap().countdown, CUT_AFTER);
        assert_eq!(r.total.state().unwrap().cut_at, Some(NOW + u64::from(CUT_AFTER)));
    }

    #[tokio::test]
    async fn a_pending_update_refuses_it() {
        let r = rig(on()).await;
        r.update.store(true, Ordering::Relaxed);
        assert!(r.total.state().unwrap().pending_update);
        for delay in [0, 600] {
            assert_eq!(r.total.schedule_at(delay, NOW).await.unwrap_err().to_string(), UPDATE);
        }
        assert_eq!(r.dev.lock().unwrap().countdown, 0);
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 0);
        assert_eq!(r.total.state().unwrap().at, None);
    }

    #[tokio::test]
    async fn an_update_that_arrives_before_the_time_set_stops_it_then() {
        let r = rig(on()).await;
        r.total.schedule_at(600, NOW).await.unwrap();
        r.update.store(true, Ordering::Relaxed);
        r.total.tick(NOW + 600).await;
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 0);
        assert_eq!(r.dev.lock().unwrap().countdown, 0);
        let s = r.total.state().unwrap();
        assert_eq!((s.at, s.error.as_deref()), (None, Some(UPDATE)));
    }

    #[tokio::test]
    async fn a_plug_that_will_not_arm_leaves_the_computer_on() {
        // It answers, but does not take the timer.
        let r = rig(Device { deaf: true, ..on() }).await;
        assert!(r.total.schedule_at(0, NOW).await.is_err());
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 0);
        assert_eq!(r.total.state().unwrap().cut_at, None);

        // It is not there at all.
        let t = Total::with(Box::new(|| panic!("must not shut down")), Box::new(|| false));
        let free = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().to_string();
        t.set_plug(Some(Plug::new(&free, "0123456789abcdef").unwrap()));
        assert!(t.schedule_at(0, NOW).await.is_err());
        assert!(t.schedule_at(600, NOW).await.is_err());
        assert_eq!(t.state().unwrap().at, None);
    }

    #[tokio::test]
    async fn a_computer_that_will_not_shut_down_disarms_the_plug() {
        let r = rig(on()).await;
        r.refuse.store(true, Ordering::Relaxed);
        let e = r.total.schedule_at(0, NOW).await.unwrap_err();
        assert!(format!("{e:#}").contains("the plug was left on"), "{e:#}");
        assert_eq!(r.dev.lock().unwrap().countdown, 0);
        assert_eq!(r.total.state().unwrap().cut_at, None);
    }

    #[tokio::test]
    async fn runs_at_the_time_set_and_not_before() {
        let r = rig(on()).await;
        r.total.schedule_at(600, NOW).await.unwrap();
        assert_eq!(r.total.state().unwrap().at, Some(NOW + 600));
        // Scheduling only checks the plug; it does not arm it.
        assert_eq!(r.dev.lock().unwrap().countdown, 0);
        r.total.tick(NOW + 599).await;
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 0);
        r.total.tick(NOW + 600).await;
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 1);
        assert_eq!(r.dev.lock().unwrap().countdown, CUT_AFTER);
        let s = r.total.state().unwrap();
        assert_eq!((s.at, s.cut_at), (None, Some(NOW + 600 + u64::from(CUT_AFTER))));
    }

    #[tokio::test]
    async fn scheduling_again_moves_it_and_cancelling_drops_it() {
        let r = rig(on()).await;
        r.total.schedule_at(600, NOW).await.unwrap();
        r.total.schedule_at(1200, NOW).await.unwrap();
        assert_eq!(r.total.state().unwrap().at, Some(NOW + 1200));
        r.total.cancel().await.unwrap();
        assert_eq!(r.total.state().unwrap().at, None);
        r.total.tick(NOW + 1200).await;
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 0);
        // More than a day ahead, and a plug that is switched off, are refused.
        assert!(r.total.schedule_at(MAX_DELAY + 1, NOW).await.is_err());
        r.dev.lock().unwrap().on = false;
        assert!(r.total.schedule_at(600, NOW).await.is_err());
        assert_eq!(r.total.state().unwrap().at, None);
    }

    #[tokio::test]
    async fn a_time_slept_through_is_skipped() {
        let r = rig(on()).await;
        r.total.schedule_at(600, NOW).await.unwrap();
        r.total.tick(NOW + 600 + LATE + 1).await;
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 0);
        let s = r.total.state().unwrap();
        assert_eq!(s.at, None);
        assert!(s.error.unwrap().contains("skipped"));
        // A little late still runs.
        r.total.schedule_at(600, NOW).await.unwrap();
        assert_eq!(r.total.state().unwrap().error, None);
        r.total.tick(NOW + 600 + LATE).await;
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn a_stalled_shutdown_keeps_the_power() {
        let r = rig(on()).await;
        r.total.schedule_at(0, NOW).await.unwrap();
        let cut = NOW + u64::from(CUT_AFTER);
        // Still running well inside the delay: nothing odd yet.
        r.total.tick(cut - guard(CUT_AFTER) - 1).await;
        assert_eq!(r.dev.lock().unwrap().countdown, CUT_AFTER);
        assert_eq!(r.total.state().unwrap().error, None);
        // Still running this close to the cut: the shutdown never happened.
        r.total.tick(cut - guard(CUT_AFTER)).await;
        assert_eq!(r.dev.lock().unwrap().countdown, 0);
        let s = r.total.state().unwrap();
        assert_eq!(s.cut_at, None);
        assert!(s.error.unwrap().contains("the plug was left on"));
        // And another can be started afterwards.
        r.total.schedule_at(0, NOW + 1000).await.unwrap();
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn cancelling_one_under_way_disarms_the_plug() {
        let r = rig(on()).await;
        r.total.schedule_at(0, NOW).await.unwrap();
        r.total.cancel().await.unwrap();
        assert_eq!(r.dev.lock().unwrap().countdown, 0);
        assert_eq!(r.total.state().unwrap().cut_at, None);
    }

    #[tokio::test]
    async fn the_wait_is_kept_in_bounds_and_used() {
        let r = rig(on()).await;
        r.total.set_cut_after(5);
        assert_eq!(r.total.state().unwrap().cut_after, 60);
        r.total.set_cut_after(100_000);
        assert_eq!(r.total.state().unwrap().cut_after, 1800);
        r.total.set_cut_after(120);
        r.total.schedule_at(0, NOW).await.unwrap();
        assert_eq!(r.dev.lock().unwrap().countdown, 120);
        assert_eq!(r.total.state().unwrap().cut_at, Some(NOW + 120));
        assert_eq!((guard(60), guard(120), guard(300), guard(1800)), (20, 24, 60, 360));
    }

    #[tokio::test]
    async fn the_wait_can_come_with_the_request() {
        let r = rig(on()).await;
        // Scheduled with its own wait: kept with the schedule, and used when the time comes.
        r.total.schedule_with(600, Some(900), NOW).await.unwrap();
        assert_eq!(*r.total.watch().borrow(), Saved { at: Some(NOW + 600), cut_after: 900 });
        r.total.tick(NOW + 600).await;
        assert_eq!(r.dev.lock().unwrap().countdown, 900);
        assert_eq!(r.total.state().unwrap().cut_at, Some(NOW + 600 + 900));

        // At once, and out of bounds: pulled in to what a shutdown can fit in.
        let r = rig(on()).await;
        r.total.schedule_with(0, Some(5), NOW).await.unwrap();
        assert_eq!(r.dev.lock().unwrap().countdown, 60);

        // A request that is refused leaves the wait as it was.
        let r = rig(on()).await;
        r.update.store(true, Ordering::Relaxed);
        assert!(r.total.schedule_with(0, Some(900), NOW).await.is_err());
        assert_eq!(r.total.state().unwrap().cut_after, CUT_AFTER);
        // Refused later on, by the plug rather than up front: the same.
        r.update.store(false, Ordering::Relaxed);
        r.dev.lock().unwrap().on = false;
        assert!(r.total.schedule_with(600, Some(900), NOW).await.is_err());
        assert_eq!(r.total.state().unwrap().cut_after, CUT_AFTER);
        r.dev.lock().unwrap().on = true;
        r.dev.lock().unwrap().deaf = true;
        assert!(r.total.schedule_with(0, Some(900), NOW).await.is_err());
        assert_eq!(r.total.state().unwrap().cut_after, CUT_AFTER);
        assert_eq!(r.shutdowns.load(Ordering::Relaxed), 0);
        r.dev.lock().unwrap().deaf = false;
        // And so does one that says nothing about it.
        r.total.set_cut_after(120);
        r.total.schedule_with(600, None, NOW).await.unwrap();
        assert_eq!(r.total.state().unwrap().cut_after, 120);
    }

    #[tokio::test]
    async fn the_schedule_survives_a_restart_and_goes_with_the_plug() {
        let r = rig(on()).await;
        r.total.set_cut_after(120);
        r.total.schedule_at(600, NOW).await.unwrap();
        let stored = serde_json::to_vec(&*r.total.watch().borrow()).unwrap();

        let again = rig(on()).await;
        again.total.load(&stored);
        assert_eq!(*again.total.watch().borrow(), Saved { at: Some(NOW + 600), cut_after: 120 });
        again.total.tick(NOW + 600).await;
        assert_eq!(again.shutdowns.load(Ordering::Relaxed), 1);

        // Garbage keeps the defaults, and a stored wait out of bounds is pulled in.
        let t = Total::with(Box::new(|| Ok(())), Box::new(|| false));
        t.load(b"not json");
        assert_eq!(*t.watch().borrow(), Saved::default());
        t.load(br#"{"at":null,"cut_after":3}"#);
        assert_eq!(t.watch().borrow().cut_after, 60);

        // Forgetting the plug drops what was scheduled for it.
        r.total.set_plug(None);
        assert_eq!(r.total.state(), None);
        assert_eq!(r.total.watch().borrow().at, None);
    }
}
