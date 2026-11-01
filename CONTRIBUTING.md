# Contributing to Tautra

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

Please send improvements to this repository as pull requests instead of publishing a separate fork, so every user gets them in one place.

## Reporting a bug

Open an issue and include:

- The Windows edition and build of both devices (for example Windows 11 24H2 Pro).
- Whether the devices connect over LAN, Wi-Fi or a VPN.
- What you did, what you expected, and what happened.
- The logs from `%LOCALAPPDATA%\Tautra\logs` on both devices. You can open the folder from **General → About & diagnostics → Open log folder**.

## Development setup

See [Building from source](README.md#building-from-source). For day-to-day work:

```powershell
cd ui
npm install
npm run tauri dev
```

Tautra needs two Windows devices to test anything that crosses the network. A desktop plus a laptop, or a desktop plus a VM, both work.

## Pull requests

- Keep each pull request focused on one change.
- Make sure `cargo clippy --workspace` passes with no warnings, and `npm run build` succeeds in `ui`.
- Code style follows the existing code: `rustfmt` defaults for Rust, and Tailwind CSS with relative units (`rem`, `%`, `vw`, `vh`) instead of `px` in the UI.
- Protocol changes must bump `PROTOCOL_VERSION` in `tautra-proto`, because devices only connect when their builds match.
- Add any new user-facing text to both the Indonesian and English strings in `ui/src/i18n/strings.ts`.

## License

By contributing, you agree that your contributions are licensed under the [GNU General Public License v3.0](LICENSE).
