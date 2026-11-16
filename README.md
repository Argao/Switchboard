# Switchboard

Control your computer from your phone: volume, brightness and contrast for every monitor, media playback, open apps and power, over an end-to-end encrypted connection on your own network. No account, no password: pair once with a QR code or a short code.

## Install

Linux (x86_64), no root needed. This puts the app, its logo and a menu entry in your home folder:

```
curl -fsSL https://raw.githubusercontent.com/aiyu-ayaan/Switchboard/master/scripts/install-linux.sh | sh
```

Add `-s -- --channel beta` for beta builds, `-s -- --version 0.0.1` for one release, or `-s -- --uninstall` to remove it. Run it again to update. The Windows installer, the `.deb` and the Android APK are on the [Releases](https://github.com/aiyu-ayaan/Switchboard/releases) page.

## RGB lighting

The lighting controls (on/off, brightness, color, effects) need [OpenRGB](https://openrgb.org) on the computer; Switchboard draws the effects and OpenRGB talks to the hardware. Without it everything else works, and the lighting controls stay hidden.

1. Install OpenRGB and check that it lists your devices.
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
