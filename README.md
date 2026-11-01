<p align="center">
  <img src="ui/public/logo.svg" alt="Tautra" width="88" />
</p>

<h1 align="center">Tautra</h1>

<p align="center">
  One keyboard and mouse, one clipboard, and drag-and-drop files across two Windows PCs.
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: GPL-3.0" src="https://img.shields.io/badge/license-GPL--3.0-blue.svg" /></a>
  <img alt="Platform: Windows 10/11" src="https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078d4.svg" />
  <img alt="Built with Rust and Tauri" src="https://img.shields.io/badge/built%20with-Rust%20%2B%20Tauri-orange.svg" />
</p>

<p align="center">
  English · <a href="README.id.md">Bahasa Indonesia</a>
</p>

---

Tautra is a software KVM for Windows. It joins two Windows devices on the same network, such as a desktop and a laptop, so they work like one desk. Push the cursor past the edge of one screen and it appears on the other. Keyboard input, the clipboard and dragged files follow it.

All traffic runs peer to peer, directly between the two devices, over QUIC with TLS. There is no cloud service, no account and no telemetry.

## Features

**Keyboard and mouse sharing**
- Move the cursor across the shared screen edge, with a visual layout editor that matches your real desk.
- Keyboard, mouse buttons, horizontal and vertical scroll, side buttons and media keys.
- Hotkeys to jump between devices and to lock the cursor. Each hotkey can be reset to its default.
- Gaming Mode: locks the cursor to the local device and turns on automatically when a full-screen app is running.
- An optional Windows service lets the cursor control UAC prompts, the lock screen and elevated windows.

**Clipboard and files**
- Clipboard sync for text, HTML, RTF, images and files.
- Large clipboard items are sent lazily, only when you paste.
- Drag and drop files from one device to the other, with progress, cancel and "show in folder".

**Connection**
- Pairing with a 6-digit PIN. Devices pin each other's certificate after that, and nothing unpaired can connect.
- Discovery over mDNS, plus manual pairing by IP address, for example over Tailscale or a VPN.
- Automatic reconnect after sleep, a network change or an IP change.
- An optional allow list that limits Tautra to the networks and adapters you choose.
- Traffic is marked as high priority with QoS (DSCP), and the agent runs at above-normal priority.

**Maintenance**
- Peer-to-peer auto update: when one device runs a newer build, the other pulls it and restarts itself.
- Log files are capped in size and cleaned up automatically.
- The UI is in Indonesian and English, with light and dark themes and a tray icon.

## Requirements

- Windows 10 version 2004 (build 19041) or newer, or Windows 11. Home, Pro, Enterprise and LTSC editions are supported.
- x64 or ARM64.
- WebView2 Runtime. It ships with Windows 11 and current Windows 10.
- Both devices on a network that can reach each other: the same LAN or Wi-Fi, or a VPN such as Tailscale.

## Getting started

1. Download the latest release from the [Releases page](https://github.com/KZMPGroup/Tautra/releases) on both devices. Use either the installer (`Tautra_x.y.z_x64-setup.exe`) or the portable zip.
2. Run the installer, or extract the zip so `tautra-ui.exe` and `tautra-agent.exe` sit in the same folder. Then start Tautra.
3. When Windows Firewall asks, allow Tautra on your network.
4. On the first device, open **Devices → Show PIN**.
5. On the second device, open **Devices → Enter PIN**, pick the first device or type its IP address, then enter the PIN.
6. Open **Screen layout** and drag the device boxes to match your desk.
7. Optional: on **Input**, install the Tautra service on both devices so the cursor can control UAC prompts and the lock screen. This needs administrator rights once.

### Default hotkeys

| Action | Hotkey |
| --- | --- |
| Move the cursor to this device | `Ctrl+Alt+F1` |
| Jump to the other device | `Ctrl+Alt+F2` |
| Lock the cursor on its current device | `Scroll Lock` |
| Toggle Gaming Mode | `Ctrl+Alt+G` |

All hotkeys can be changed on the **Input** page.

### Network

| Port | Protocol | Purpose |
| --- | --- | --- |
| 47800 (default) | UDP | QUIC connection between devices |
| 5353 | UDP | mDNS discovery |

### Files and folders

| What | Where |
| --- | --- |
| Settings and screen layout | `%APPDATA%\Tautra` |
| Trusted devices, identity and logs | `%LOCALAPPDATA%\Tautra` |
| Service (when installed) | `%ProgramData%\Tautra\service` |

## Building from source

You need:

- Rust 1.85 or newer, with the MSVC toolchain (`x86_64-pc-windows-msvc`)
- Visual Studio Build Tools with the "Desktop development with C++" workload
- Node.js LTS

Then:

```powershell
git clone https://github.com/KZMPGroup/Tautra.git
cd Tautra\ui
npm install
npm run tauri build -- --no-bundle
```

This writes `tautra-ui.exe` and `tautra-agent.exe` to `target\release`. Copy both to the same folder.

To build the installer, run `npm run tauri build` without `--no-bundle`. The installer is written to `target\release\bundle\nsis`.

For development, run `npm run tauri dev` in `ui`. It builds the agent first and starts the UI with hot reload.

## Architecture

Tautra has two processes:

- **`tautra-agent`** runs in the background and does all the work: networking, input hooks, clipboard, file transfer and updates.
- **`tautra-ui`** is the Tauri and React window and tray icon.

They talk over JSON-RPC on a named pipe. Closing the window does not stop sharing.

```
crates/
├─ tautra-agent      background process: wires everything together
├─ tautra-net        QUIC/TLS sessions, pairing, mDNS, network watch, auto update
├─ tautra-proto      wire protocol (postcard)
├─ tautra-input      low-level keyboard/mouse hooks and input injection
├─ tautra-clipboard  clipboard listener and delayed rendering
├─ tautra-transfer   file transfer and drag-and-drop
├─ tautra-service    optional Windows service for secure-desktop input
├─ tautra-ipc        JSON-RPC over named pipe between agent and UI
└─ tautra-common     config, paths, logging, build id
ui/
├─ src               React + TypeScript + Tailwind CSS
└─ src-tauri         Tauri shell (tautra-ui.exe)
```

The original product spec is in [docs/PRD.md](docs/PRD.md) and the development log is in [docs/PROGRESS.md](docs/PROGRESS.md). Both are written in Indonesian.

## Security

- Devices pair with a short-lived PIN. After that each side pins the other's certificate fingerprint.
- Every connection is encrypted with TLS 1.3 over QUIC. Devices that are not trusted cannot connect.
- Updates are only accepted from a trusted, paired device.
- Nothing leaves your network except traffic to the other device you paired.

To report a vulnerability, see [SECURITY.md](SECURITY.md).

## Contributing

Issues and pull requests are welcome. Please send changes here as pull requests, not as a separate fork project. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Copyright (C) 2026 KZMP Group

Tautra is free software, released under the [GNU General Public License v3.0](LICENSE). You can use it for free, personally or at work. You can study and change the source code.

If you distribute Tautra or a modified version of it, you must release the full source code under the same GPL-3.0 license. You must also keep the copyright notice. Closed-source or proprietary versions are not allowed.

Improvements are best sent back to this repository as pull requests, so every user gets them in one place. Please do not publish a separate rebranded copy.
