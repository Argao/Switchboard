# Switchboard

Control your computer from your phone: volume, brightness and contrast for every monitor, media playback, open apps and power, over an end-to-end encrypted connection on your own network. No account, no password: pair once with a QR code or a short code.

## Install

Linux (x86_64), no root needed. This puts the app, its logo and a menu entry in your home folder:

```
curl -fsSL https://raw.githubusercontent.com/Argao/Switchboard/master/scripts/install-linux.sh | sh
```

Add `-s -- --channel beta` for beta builds, `-s -- --version 0.0.1` for one release, or `-s -- --uninstall` to remove it. Run it again to update. The Windows installer, the `.deb` and the Android APK are on the [Releases](https://github.com/Argao/Switchboard/releases) page.

### Requirements

To run the installed app. The desktop app bundles Electron and the daemon, so there is nothing to install for Node or Rust.

**Linux (x86_64)**

- `curl` or `wget`, and `sha256sum` (or `shasum`), used by the installer.
- **FUSE 2 (`libfuse.so.2`)**: AppImages will not start without it (`dlopen(): error loading libfuse.so.2`).
  - Debian/Ubuntu 24.04 and newer: `sudo apt install libfuse2t64` (older releases: `libfuse2`)
  - Fedora: `sudo dnf install fuse-libs`
  - Arch: `sudo pacman -S fuse2`
- `systemd` (`loginctl`, `systemctl`) for lock, sleep, shutdown and restart, and a D-Bus session (media control through MPRIS).
- PulseAudio or PipeWire with `pactl` (`pulseaudio-utils`, or `pipewire-pulse` plus `pulseaudio-utils`) for volume.
- Monitor brightness and contrast (DDC/CI) need the I2C device nodes: `sudo modprobe i2c-dev`, then your user in the `i2c` group (`sudo usermod -aG i2c $USER`, log out and in). Keep it across reboots with `echo i2c-dev | sudo tee /etc/modules-load.d/i2c-dev.conf`.
- A Secret Service (GNOME Keyring or KWallet) to store the pairing key.
- An X11 session for keyboard and mouse input; on Wayland the clipboard works but input needs XWayland.

**Windows 10 or 11 (x64)**

- The installer from the [Releases](https://github.com/Argao/Switchboard/releases) page. Nothing else: audio, brightness (DDC/CI), media and power use what Windows already has.
- A monitor with DDC/CI turned on in its own menu for brightness and contrast.
- For the Apps area and total shutdown, run Switchboard as the same user that owns the windows (programs running as administrator can only be changed at the computer).

**Both**

- A phone on the same network (mDNS discovery), with the Android app.
- Optional: [OpenRGB](https://openrgb.org) for the lighting controls (see below).

## RGB lighting

The lighting controls (on/off, brightness, color, effects) need [OpenRGB](https://openrgb.org) on the computer; Switchboard draws the effects and OpenRGB talks to the hardware. Without it everything else works, and the lighting controls stay hidden.

1. Install OpenRGB and check that it lists your devices. On Debian/Ubuntu: `sudo apt install openrgb` (it ships the udev rules, so devices work without root; replug USB devices or reboot after installing). To reach RAM and motherboard lighting over SMBus, also load the I2C module: `sudo modprobe i2c-dev`, and to keep it across reboots `echo i2c-dev | sudo tee /etc/modules-load.d/i2c-dev.conf`.
2. Turn on its SDK server: the **SDK Server** tab, then **Start Server**, or start OpenRGB with `--server`. Keep the default port, 6742.
3. Leave OpenRGB running. Switchboard finds it within a few seconds and shows "RGB lighting" on the desktop app and the phone.

Switchboard leaves your OpenRGB setup alone until you change a lighting control for the first time.

## Apps

The phone lists the windows open on the computer and can close one, end its program, or make it cover a screen of your choice (and put it back). This works on Windows for now; on other systems the Apps area stays hidden. Programs running as administrator can only be changed at the computer.

## Total shutdown

With the computer plugged into a Tuya smart plug (the kind set up with the Smart Life app, sold under many brands), the phone can shut it down and then have the plug cut the power, at once or on a timer. The plug counts the wait by itself, since by then the computer is off.

Set the plug up in the desktop app, under Settings, Total shutdown:

- **From Smart Life**: enter the User Code from the app (Me, settings, Account and Security), scan the QR code with it, and pick the plug. Switchboard reads the plug's key from your account once and then finds the plug on your network. This uses the sign-in Tuya provides for Home Assistant, so it may stop working; the next way always does.
- **Enter by hand**: the plug's address and its 16-character local key, read from a Tuya developer account.

After that the computer talks to the plug directly, with no cloud. Only plugs on protocol 3.5 are supported for now. Programs still open are closed without asking, so unsaved work is lost. Everything else on the same plug loses power too, and the computer cannot be switched back on from the phone. A total shutdown is refused while Windows has an update waiting to install.

## Layout

```
backend/        Rust daemon (desktop host): WebSocket server, SQLite, mDNS, system controls
core/           Rust protocol + crypto, shared by the daemon and the Android app (JNI)
frontend/       Electron + React + Tailwind desktop app (tray, start at login)
mobile/         Android app (Kotlin + Jetpack Compose, Material 3 Expressive)
design-system/  Shared M3 colour scheme, brand mark, design rules
development/    Docs submodule (architecture, pairing protocol, roadmap, commit rules)
```

## Develop

Build requirements, on both systems: Node.js 20+ with pnpm (`corepack enable`), the Rust toolchain from `rust-toolchain.toml` (install it with [rustup](https://rustup.rs)), and Git with submodules (`git submodule update --init`).

- **Linux**: a C compiler and `pkg-config` (the bundled SQLite is built from source): `sudo apt install build-essential pkg-config` (Fedora: `sudo dnf groupinstall "Development Tools"`). To package: `dpkg`, `fakeroot`, `rpm` for the `.deb`.
- **Windows**: Visual Studio Build Tools 2022 with the "Desktop development with C++" workload (MSVC and the Windows SDK), and the `x86_64-pc-windows-msvc` Rust target (rustup's default there).
- **Android app** (either system): JDK 17, Android SDK 37 (minSdk 26) and NDK, `cargo-ndk` (`cargo install cargo-ndk --locked`), the Android Rust targets (`rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`), and `ANDROID_NDK_HOME`. Gradle comes from the wrapper.

```
pnpm install
cargo build -p switchboard-backend           # daemon, picked up by the desktop app
pnpm --filter frontend dev                   # Vite on :5273
pnpm --filter frontend dev:electron          # Electron + dev daemon on port 9428
cargo test --workspace                       # crypto, protocol, handshake end to end

cd mobile && ./gradlew :app:assembleDebug -Pabis=x86_64   # needs cargo-ndk and ANDROID_NDK_HOME
```

The debug Android build (`com.aiyu.switchboard.dev`) talks to the dev daemon on
port 9428; on an emulator run `adb reverse tcp:9428 tcp:9428`.

See [`development/devdocs/architecture.md`](development/devdocs/architecture.md) and
[`development/devdocs/security-pairing.md`](development/devdocs/security-pairing.md).
