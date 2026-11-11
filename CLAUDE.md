# Panduan untuk AI yang mengerjakan Tautra

Baca `docs/PROGRESS.md` dulu, lanjutkan dari bagian yang belum selesai. Balas dalam bahasa Indonesia.

## Mode build

- **DEV**: semua build lokal untuk diuji pengguna. Dibangun dengan `npm run build:dev` di `ui` (fitur cargo `devlog`). Log mencatat semua kejadian secara lengkap (trace untuk semua crate `tautra_*`), termasuk hook input, relay layanan untuk jendela admin/UAC, kursor di kedua perangkat, injeksi, dan setiap kegagalan. Tidak boleh ada kegagalan yang diam-diam: setiap `Err` dicatat minimal `warn`.
- **PRODUCTION**: hanya build yang dirilis ke GitHub. Dibangun dengan `npm run build:prod` (log level `info`).
- Lokasi log: agent di `%LOCALAPPDATA%\Tautra\logs\agent.YYYY-MM-DD.log`, layanan dan worker di `C:\ProgramData\Tautra\logs\service.YYYY-MM-DD.log` (waktu UTC). Baris `logging aktif` memuat `mode=dev|production`.
- Saat menambah fitur atau memperbaiki bug, tambahkan log `trace`/`debug` di jalur yang diubah supaya build DEV merekam alur lengkapnya.

## Alur kerja

- Naikkan versi di `Cargo.toml`, `Cargo.lock`, `ui/package.json`, `ui/package-lock.json`, `ui/src-tauri/tauri.conf.json` pada setiap perubahan. Build id harus sama di semua perangkat agar bisa terhubung; laptop memperbarui diri lewat auto-update p2p.
- Build dan pasang di PC pengguna dulu (`%LOCALAPPDATA%\Tautra`). Push ke GitHub dan rilis hanya setelah pengguna menguji dan menyatakan OK.
- Catat setiap perubahan di `docs/PROGRESS.md`.

## Animasi perpindahan kursor

Aturan dan cara kerja animasi masuk/keluar ada di `docs/ANIMASI.md`. Baca sebelum mengubah atau menambah gaya animasi.

## Gaya kode

- Tanpa komentar di kode. Pesan log dan teks pengguna dalam bahasa Indonesia.
- Tailwind memakai satuan rem/%/vw/vh, bukan px. Jangan ubah setup build Tailwind.
- Tidak perlu membuat modul tes atau menjalankan tes di akhir.
