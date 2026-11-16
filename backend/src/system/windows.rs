//! The programs open on the desktop: listing their windows, closing them,
//! ending them, and making one cover a screen of the person's choice.
//!
//! There is no one "full screen" a program can be told to enter; each draws
//! its own. So a window is made to cover its screen from outside: its frame
//! is taken off and it is sized to the screen, the way browsers do it for
//! themselves. What it looked like before is kept, to put it back.
//!
//! Only Windows for now. Elsewhere `list` answers `None` and the phone hides
//! the whole area.

use anyhow::{Result, anyhow};
use switchboard_core::protocol::{AppWindow, Screen};

/// What the desktop says about a top-level window, for deciding whether it
/// is one a person would call an open program.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub visible: bool,
    pub title: String,
    /// Belongs to another window: a dialog, a tooltip, a menu.
    pub owned: bool,
    /// Marked as a helper that stays off the taskbar.
    pub tool: bool,
    /// Kept by the system but not drawn (a suspended Store app, another virtual desktop).
    pub cloaked: bool,
    pub class: String,
}

/// The desktop itself and the taskbar are windows too.
const SHELL: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

pub fn listable(f: &Facts) -> bool {
    f.visible && !f.owned && !f.tool && !f.cloaked && !f.title.trim().is_empty() && !SHELL.contains(&f.class.as_str())
}

/// The program's name from the path of what is running: `C:\…\firefox.exe` is `firefox`.
pub fn app_name(exe: &str) -> String {
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    let stem = file.rsplit_once('.').map_or(file, |(stem, ext)| if ext.eq_ignore_ascii_case("exe") { stem } else { file });
    if stem.is_empty() { "Unknown".into() } else { stem.into() }
}

/// What a screen is called: the monitor's own name, or its place in the list.
pub fn screen_name(friendly: Option<&str>, index: usize) -> String {
    match friendly.map(str::trim) {
        Some(n) if !n.is_empty() => n.into(),
        _ => format!("Screen {}", index + 1),
    }
}

/// A window id is the handle the desktop gave it, as a number.
pub fn parse_id(id: &str) -> Result<isize> {
    id.parse::<isize>().ok().filter(|&h| h != 0).ok_or_else(|| anyhow!("not a window: {id}"))
}

pub use imp::{close, fullscreen, kill, list};

#[cfg(windows)]
mod imp {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::{LazyLock, Mutex};

    use windows::Win32::Devices::Display::{
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
        DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
    };
    use windows::Win32::Foundation::{BOOL, CloseHandle, ERROR_SUCCESS, HWND, LPARAM, RECT, TRUE, WPARAM};
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW, MonitorFromWindow};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess};
    use windows::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_MENU, keybd_event};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, EnumWindows, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetWindow, GetWindowLongPtrW, GetWindowPlacement, GetWindowTextW, GetWindowThreadProcessId,
        HWND_TOP, IsIconic, IsWindow, IsWindowVisible, IsZoomed, MONITORINFOF_PRIMARY, PostMessageW, SW_RESTORE, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
        SWP_NOZORDER, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPlacement, SetWindowPos, ShowWindow, WINDOWPLACEMENT, WM_CLOSE, WS_CAPTION, WS_EX_CLIENTEDGE,
        WS_EX_DLGMODALFRAME, WS_EX_STATICEDGE, WS_EX_TOOLWINDOW, WS_EX_WINDOWEDGE, WS_MAXIMIZE, WS_MINIMIZE, WS_THICKFRAME,
    };
    use windows::core::PWSTR;

    const REFUSED: &str = "Windows would not allow it. A program running as administrator can only be changed at the computer.";

    /// How a window looked before it was made to cover a screen.
    struct Before {
        style: isize,
        ex: isize,
        place: WINDOWPLACEMENT,
    }

    /// The windows put in full screen from here, by handle.
    static FULL: LazyLock<Mutex<HashMap<isize, Before>>> = LazyLock::new(Mutex::default);

    fn hwnd(h: isize) -> HWND {
        HWND(h as *mut c_void)
    }

    fn text(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    struct Monitor {
        id: String,
        rect: RECT,
        primary: bool,
    }

    fn monitor(m: HMONITOR) -> Option<Monitor> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: `info` is the extended structure its own size field announces.
        unsafe { GetMonitorInfoW(m, &mut info as *mut _ as *mut MONITORINFO) }.as_bool().then(|| Monitor {
            id: text(&info.szDevice),
            rect: info.monitorInfo.rcMonitor,
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        })
    }

    fn monitors() -> Vec<Monitor> {
        unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, out: LPARAM) -> BOOL {
            // SAFETY: `out` is the vector `monitors` passed, alive for the whole enumeration.
            let out = unsafe { &mut *(out.0 as *mut Vec<Monitor>) };
            out.extend(monitor(m));
            TRUE
        }
        let mut out: Vec<Monitor> = Vec::new();
        // SAFETY: the callback only runs during this call, while `out` is alive.
        unsafe {
            let _ = EnumDisplayMonitors(HDC::default(), None, Some(each), LPARAM(&mut out as *mut _ as isize));
        }
        // The main screen first, then left to right.
        out.sort_by_key(|m| (!m.primary, m.rect.left, m.rect.top));
        out
    }

    /// The name each monitor gives itself ("DELL U2720Q"), by the device name
    /// the rest of the desktop knows its screen by (`\\.\DISPLAY1`).
    fn friendly_names() -> HashMap<String, String> {
        let mut names = HashMap::new();
        let (mut paths_len, mut modes_len) = (0u32, 0u32);
        // SAFETY: both buffers are sized as the first call asked, and each
        // request header carries the size and kind of the structure it sits in.
        unsafe {
            if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths_len, &mut modes_len) != ERROR_SUCCESS {
                return names;
            }
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); paths_len as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); modes_len as usize];
            if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut paths_len, paths.as_mut_ptr(), &mut modes_len, modes.as_mut_ptr(), None) != ERROR_SUCCESS {
                return names;
            }
            for p in paths.iter().take(paths_len as usize) {
                let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                    header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                        size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                        adapterId: p.sourceInfo.adapterId,
                        id: p.sourceInfo.id,
                    },
                    ..Default::default()
                };
                let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                    header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                        size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                        adapterId: p.targetInfo.adapterId,
                        id: p.targetInfo.id,
                    },
                    ..Default::default()
                };
                if DisplayConfigGetDeviceInfo(&mut source.header) == 0 && DisplayConfigGetDeviceInfo(&mut target.header) == 0 {
                    names.insert(text(&source.viewGdiDeviceName), text(&target.monitorFriendlyDeviceName));
                }
            }
        }
        names
    }

    fn screens() -> Vec<Screen> {
        let names = friendly_names();
        monitors()
            .into_iter()
            .enumerate()
            .map(|(i, m)| Screen {
                name: screen_name(names.get(&m.id).map(String::as_str), i),
                id: m.id,
                primary: m.primary,
                width: (m.rect.right - m.rect.left).max(0) as u32,
                height: (m.rect.bottom - m.rect.top).max(0) as u32,
            })
            .collect()
    }

    fn facts(h: HWND) -> Facts {
        let mut title = [0u16; 512];
        let mut class = [0u16; 256];
        let mut cloaked = 0u32;
        // SAFETY: every buffer is passed with its own length, and a handle
        // that has gone stale only makes these calls fail.
        unsafe {
            GetWindowTextW(h, &mut title);
            GetClassNameW(h, &mut class);
            let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut c_void, size_of::<u32>() as u32);
            Facts {
                visible: IsWindowVisible(h).as_bool(),
                title: text(&title),
                owned: GetWindow(h, GW_OWNER).is_ok_and(|o| !o.is_invalid()),
                tool: GetWindowLongPtrW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0,
                cloaked: cloaked != 0,
                class: text(&class),
            }
        }
    }

    fn pid_of(h: HWND) -> u32 {
        let mut pid = 0u32;
        // SAFETY: `pid` outlives the call.
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        pid
    }

    fn exe_of(pid: u32) -> Option<String> {
        // SAFETY: the handle is closed before returning, and the buffer is passed with its length.
        unsafe {
            let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let r = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
            let _ = CloseHandle(p);
            r.ok()?;
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        }
    }

    /// The process a window stands for, and its name. Store apps are drawn
    /// inside a frame that one shared host process owns; the app itself is
    /// the process of the window inside that frame.
    fn owner(h: HWND) -> (u32, String) {
        let pid = pid_of(h);
        let name = exe_of(pid).map(|e| app_name(&e)).unwrap_or_else(|| "Unknown".into());
        if !name.eq_ignore_ascii_case("ApplicationFrameHost") {
            return (pid, name);
        }
        unsafe extern "system" fn each(child: HWND, found: LPARAM) -> BOOL {
            // SAFETY: `found` is the pair `owner` passed, alive for the whole enumeration.
            let found = unsafe { &mut *(found.0 as *mut (u32, u32)) };
            let pid = pid_of(child);
            if pid != 0 && pid != found.0 {
                found.1 = pid;
                return BOOL(0);
            }
            TRUE
        }
        let mut found = (pid, 0u32);
        // SAFETY: the callback only runs during this call, while `found` is alive.
        unsafe {
            let _ = EnumChildWindows(h, Some(each), LPARAM(&mut found as *mut _ as isize));
        }
        match found.1 {
            0 => (pid, name),
            inner => (inner, exe_of(inner).map(|e| app_name(&e)).unwrap_or(name)),
        }
    }

    fn handles() -> Vec<HWND> {
        unsafe extern "system" fn each(h: HWND, out: LPARAM) -> BOOL {
            // SAFETY: `out` is the vector `handles` passed, alive for the whole enumeration.
            unsafe { &mut *(out.0 as *mut Vec<HWND>) }.push(h);
            TRUE
        }
        let mut out: Vec<HWND> = Vec::new();
        // SAFETY: the callback only runs during this call, while `out` is alive.
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut out as *mut _ as isize));
        }
        out
    }

    pub fn list() -> Option<(Vec<AppWindow>, Vec<Screen>)> {
        let mut full = FULL.lock().unwrap();
        // Forget the ones that were closed while covering a screen.
        // SAFETY: asking whether a handle is still a window is safe for any value.
        full.retain(|&h, _| unsafe { IsWindow(hwnd(h)) }.as_bool());
        let windows = handles()
            .into_iter()
            .filter_map(|h| {
                let f = facts(h);
                listable(&f).then(|| AppWindow {
                    id: (h.0 as isize).to_string(),
                    app: owner(h).1,
                    title: f.title,
                    // SAFETY: a stale handle only makes this return the nearest screen.
                    screen: monitor(unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) }).map(|m| m.id),
                    fullscreen: full.contains_key(&(h.0 as isize)),
                })
            })
            .collect();
        Some((windows, screens()))
    }

    /// Only the windows `list` shows may be acted on, whatever number a phone sends.
    fn open(id: &str) -> Result<HWND> {
        let h = hwnd(parse_id(id)?);
        // SAFETY: asking whether a handle is still a window is safe for any value.
        anyhow::ensure!(unsafe { IsWindow(h) }.as_bool() && listable(&facts(h)), "That window is no longer open");
        Ok(h)
    }

    pub fn close(id: &str) -> Result<()> {
        let h = open(id)?;
        // SAFETY: posting a message does not wait for the window to answer.
        unsafe { PostMessageW(h, WM_CLOSE, WPARAM(0), LPARAM(0)) }.map_err(|_| anyhow!(REFUSED))
    }

    pub fn kill(id: &str) -> Result<()> {
        let (pid, _) = owner(open(id)?);
        anyhow::ensure!(pid != 0 && pid != std::process::id(), "That program cannot be ended from here");
        // SAFETY: the handle is closed before returning.
        unsafe {
            let p = OpenProcess(PROCESS_TERMINATE, false, pid).map_err(|_| anyhow!(REFUSED))?;
            let r = TerminateProcess(p, 1);
            let _ = CloseHandle(p);
            r.map_err(|_| anyhow!(REFUSED))
        }
    }

    /// Brings a window to the front. Windows only lets the program in front
    /// hand that over, unless the person has just pressed a key: so when it
    /// refuses, Alt is tapped and it is asked again.
    fn focus(h: HWND) {
        // SAFETY: plain calls on a window handle; a stale one only makes them fail.
        unsafe {
            if !SetForegroundWindow(h).as_bool() {
                keybd_event(VK_MENU.0 as u8, 0, Default::default(), 0);
                keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
                let _ = SetForegroundWindow(h);
            }
        }
    }

    pub fn fullscreen(id: &str, screen: Option<&str>, on: bool) -> Result<()> {
        let h = open(id)?;
        let key = h.0 as isize;
        let mut full = FULL.lock().unwrap();
        // SAFETY: plain calls on a window handle that `open` has just checked;
        // `place` carries its own length.
        unsafe {
            if !on {
                let was = full.remove(&key).ok_or_else(|| anyhow!("That window is not in full screen"))?;
                SetWindowLongPtrW(h, GWL_STYLE, was.style);
                SetWindowLongPtrW(h, GWL_EXSTYLE, was.ex);
                let _ = SetWindowPlacement(h, &was.place);
                return SetWindowPos(h, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED).map_err(|_| anyhow!(REFUSED));
            }

            let to = match screen {
                Some(s) => monitors().into_iter().find(|m| m.id == s).ok_or_else(|| anyhow!("That screen is no longer connected"))?,
                None => monitor(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST)).ok_or_else(|| anyhow!("no screen found"))?,
            };
            // Moving a window that already covers a screen keeps what it looked like before the first time.
            if !full.contains_key(&key) {
                let mut place = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
                GetWindowPlacement(h, &mut place).map_err(|_| anyhow!(REFUSED))?;
                full.insert(key, Before { style: GetWindowLongPtrW(h, GWL_STYLE), ex: GetWindowLongPtrW(h, GWL_EXSTYLE), place });
            }
            // A minimised or maximised window does not take a new size.
            if IsIconic(h).as_bool() || IsZoomed(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            let was = &full[&key];
            SetWindowLongPtrW(h, GWL_STYLE, was.style & !((WS_CAPTION | WS_THICKFRAME | WS_MINIMIZE | WS_MAXIMIZE).0 as isize));
            SetWindowLongPtrW(h, GWL_EXSTYLE, was.ex & !((WS_EX_DLGMODALFRAME | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_STATICEDGE).0 as isize));
            let r = &to.rect;
            if SetWindowPos(h, HWND_TOP, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_FRAMECHANGED | SWP_NOOWNERZORDER | SWP_SHOWWINDOW).is_err() {
                full.remove(&key);
                return Err(anyhow!(REFUSED));
            }
            focus(h);
        }
        Ok(())
    }

    /// Drives a real window on this desktop, so it is run by hand:
    /// `cargo test --bins -- --ignored windows`.
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::process::{Child, Command};
        use std::time::{Duration, Instant};
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

        /// A plain resizable window in a process of its own.
        fn spawn(title: &str) -> Child {
            let script = format!(
                "Add-Type -AssemblyName System.Windows.Forms; $f = New-Object Windows.Forms.Form; $f.Text = '{title}'; $f.Width = 400; $f.Height = 300; [Windows.Forms.Application]::Run($f)"
            );
            Command::new("powershell").args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script]).spawn().unwrap()
        }

        fn wait<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
            let until = Instant::now() + Duration::from_secs(20);
            loop {
                if let Some(v) = f() {
                    return v;
                }
                assert!(Instant::now() < until, "timed out waiting for {what}");
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        fn find(title: &str) -> Option<AppWindow> {
            list().unwrap().0.into_iter().find(|w| w.title == title)
        }

        fn rect(id: &str) -> RECT {
            let mut r = RECT::default();
            unsafe { GetWindowRect(hwnd(parse_id(id).unwrap()), &mut r).unwrap() };
            r
        }

        #[test]
        #[ignore = "opens windows on this desktop"]
        fn covers_a_screen_puts_it_back_closes_and_ends() {
            let title = format!("switchboard-test-{}", std::process::id());
            let mut child = spawn(&title);
            let w = wait("the window", || find(&title));
            assert_eq!(w.app, "powershell");
            assert!(!w.fullscreen);
            let (_, screens) = list().unwrap();
            assert!(screens.iter().filter(|s| s.primary).count() == 1, "{screens:?}");
            assert!(w.screen.as_ref().is_some_and(|id| screens.iter().any(|s| &s.id == id)));

            // Onto every screen in turn, covering each exactly.
            let before = rect(&w.id);
            for (s, m) in screens.iter().zip(monitors()) {
                fullscreen(&w.id, Some(&s.id), true).unwrap();
                let now = wait("full screen", || Some(rect(&w.id)).filter(|r| *r == m.rect));
                assert_eq!((now.right - now.left) as u32, s.width);
                let listed = find(&title).unwrap();
                assert!(listed.fullscreen);
                assert_eq!(listed.screen.as_deref(), Some(s.id.as_str()));
            }
            // And back to where and what it was, however many screens it visited.
            fullscreen(&w.id, None, false).unwrap();
            wait("the old size", || Some(rect(&w.id)).filter(|r| *r == before));
            assert!(!find(&title).unwrap().fullscreen);
            assert!(fullscreen(&w.id, None, false).is_err());
            assert!(fullscreen(&w.id, Some("no such screen"), true).is_err());

            // Closing is asking: the program ends by itself.
            close(&w.id).unwrap();
            wait("the program to end", || child.try_wait().unwrap());
            assert!(find(&title).is_none());
            assert!(close(&w.id).is_err());

            // Ending is not asking.
            let mut child = spawn(&title);
            let w = wait("the second window", || find(&title));
            kill(&w.id).unwrap();
            let status = wait("the program to be ended", || child.try_wait().unwrap());
            assert_eq!(status.code(), Some(1));
            assert!(kill(&w.id).is_err());
        }

        #[test]
        fn only_listed_windows_can_be_touched() {
            // The desktop is a real window, and not one of the programs.
            let desktop = handles().into_iter().find(|&h| facts(h).class == "Progman").map(|h| (h.0 as isize).to_string());
            for id in desktop.iter().map(String::as_str).chain(["0", "1", "abc", ""]) {
                assert!(close(id).is_err(), "{id}");
                assert!(kill(id).is_err(), "{id}");
                assert!(fullscreen(id, None, true).is_err(), "{id}");
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    const NO: &str = "Managing windows is not available on this computer yet";

    pub fn list() -> Option<(Vec<AppWindow>, Vec<Screen>)> {
        None
    }
    pub fn close(_: &str) -> Result<()> {
        Err(anyhow!(NO))
    }
    pub fn kill(_: &str) -> Result<()> {
        Err(anyhow!(NO))
    }
    pub fn fullscreen(_: &str, _: Option<&str>, _: bool) -> Result<()> {
        Err(anyhow!(NO))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Facts {
        Facts { visible: true, title: "Inbox - Mail".into(), class: "Chrome_WidgetWin_1".into(), ..Default::default() }
    }

    #[test]
    fn lists_what_a_person_would_call_open() {
        assert!(listable(&window()));
        assert!(!listable(&Facts { visible: false, ..window() }));
        assert!(!listable(&Facts { owned: true, ..window() }));
        assert!(!listable(&Facts { tool: true, ..window() }));
        assert!(!listable(&Facts { cloaked: true, ..window() }));
        assert!(!listable(&Facts { title: "  ".into(), ..window() }));
        // The desktop and the taskbar have titles and are visible.
        for class in SHELL {
            assert!(!listable(&Facts { class: class.into(), title: "Program Manager".into(), ..window() }), "{class}");
        }
    }

    #[test]
    fn names_programs_by_what_runs() {
        assert_eq!(app_name(r"C:\Program Files\Mozilla Firefox\firefox.exe"), "firefox");
        assert_eq!(app_name(r"C:\Tools\My.App.EXE"), "My.App");
        assert_eq!(app_name("/usr/bin/gnome-terminal-server"), "gnome-terminal-server");
        assert_eq!(app_name("/opt/app-1.2"), "app-1.2");
        assert_eq!(app_name(""), "Unknown");
        assert_eq!(app_name(".exe"), "Unknown");
    }

    #[test]
    fn names_screens() {
        assert_eq!(screen_name(Some("DELL U2720Q"), 0), "DELL U2720Q");
        // A laptop panel or a generic monitor says nothing about itself.
        assert_eq!(screen_name(Some(" "), 0), "Screen 1");
        assert_eq!(screen_name(None, 2), "Screen 3");
    }

    #[test]
    fn window_ids_are_numbers() {
        assert_eq!(parse_id("66312").unwrap(), 66312);
        for bad in ["", "0", "abc", "12 ", "0x10", "99999999999999999999999"] {
            assert!(parse_id(bad).is_err(), "{bad}");
        }
    }
}
