# PRD — Tautra

> Catatan: PRD asli memakai nama kerja **Sambung**. Proyek ini bernama **Tautra**, sehingga seluruh penamaan (crate, path konfigurasi/log, nama pipe, layanan mDNS, folder unduhan) memakai `Tautra` / `tautra-`. Isi spesifikasi selain penamaan tidak diubah.

Aplikasi Windows untuk menyatukan laptop dan PC dalam satu jaringan lokal: satu keyboard & mouse, clipboard & file bersama, dan window yang bisa diseret antar perangkat seperti dual monitor.

---

## 1. Latar Belakang & Tujuan

Pengguna punya 2 perangkat Windows:
- **PC (utama)**: keyboard & mouse fisik terpasang di sini, dipakai di rumah, terhubung LAN kabel.
- **Laptop (portable)**: sering dibawa keluar, di rumah terhubung ke jaringan yang sama (WiFi atau kabel).

Tujuan:
1. Satu keyboard & mouse (milik PC) mengontrol kedua perangkat. Kursor pindah dengan menggeser ke tepi layar.
2. Clipboard (teks, gambar, file) tersinkron otomatis.
3. File bisa di-drag and drop antar perangkat.
4. Window aplikasi bisa diseret dari layar laptop ke layar PC (dan sebaliknya) dengan mulus, seperti dual monitor. Aplikasi tetap berjalan di perangkat asalnya, tampilannya di-stream.
5. Otomatis tersambung saat kedua perangkat berada di jaringan yang sama, tanpa interaksi pengguna.
6. Saat laptop di luar rumah, Tautra idle dan ringan (hampir 0% CPU, tidak mengganggu baterai).
7. Latensi sangat rendah, nyaman untuk kerja dan game santai.

## 2. Non-Goals

- Migrasi proses/program yang sedang berjalan antar mesin (secara teknis tidak mungkin di Windows).
- Koneksi lewat internet / di luar jaringan lokal (versi 1 hanya LAN).
- Dukungan macOS / Linux (versi 1 hanya Windows 10 21H2+ dan Windows 11).
- Membuat driver kernel sendiri.
- Lebih dari 2 perangkat (arsitektur boleh siap untuk >2, tapi tidak wajib di v1).

## 3. Istilah

- **Host input**: perangkat yang punya keyboard/mouse fisik (PC).
- **Origin**: perangkat tempat sebuah aplikasi benar-benar berjalan.
- **Viewer**: perangkat yang menampilkan window hasil stream.
- **Virtual monitor**: monitor palsu yang dibuat di origin melalui driver Indirect Display (IddCx), merepresentasikan layar perangkat lain.
- **Proxy window**: window tanpa border di viewer yang menampilkan stream dari satu window di origin.
- **Seamless mode**: mode di mana window dari origin tampil sebagai window individual di desktop viewer (bukan satu layar penuh).

## 4. Pengalaman Pengguna (User Stories)

1. Saya pulang, membuka laptop. Dalam ≤5 detik setelah laptop terhubung ke WiFi rumah, Tautra tersambung ke PC tanpa saya klik apa pun, dan muncul notifikasi kecil "Tersambung ke PC".
2. Saya menggeser mouse ke tepi kanan layar PC, kursor muncul di laptop. Keyboard ikut mengetik di laptop.
3. Saya copy teks/gambar di laptop, lalu paste di PC.
4. Saya drag file dari Explorer laptop, lalu drop ke desktop/folder di PC.
5. Saya drag window Chrome dari laptop melewati tepi layar, window itu muncul dan bisa dipakai di layar PC seperti window biasa.
6. Saya mencabut / membawa pergi laptop. Semua window laptop yang sedang tampil di PC otomatis kembali ke layar laptop, tidak ada yang hilang.
7. Saya bermain game di PC. Saya menekan hotkey "Gaming Mode" sehingga kursor terkunci di layar PC dan overhead Tautra minimal.
8. Saya mengatur posisi layar (laptop di kiri/kanan/atas/bawah PC) dengan drag di halaman pengaturan, seperti pengaturan Display Windows.

## 5. Arsitektur

### 5.1 Stack Teknologi

| Komponen | Teknologi |
|---|---|
| Core (jaringan, input, capture, encode, decode, render) | Rust (crate `windows` / windows-rs) |
| Transport | QUIC via crate `quinn` (reliable streams untuk kontrol & file, datagram untuk video & input) |
| Discovery | mDNS via crate `mdns-sd` |
| Kripto pairing | X25519 + sertifikat self-signed per perangkat, pinned saat pairing |
| Capture | Windows.Graphics.Capture (WGC), zero-copy ke D3D11 texture |
| Encode | Media Foundation hardware MFT (NVENC / QuickSync / AMF), fallback software H.264 |
| Decode & render | Media Foundation D3D11 decoder + Direct3D 11 swapchain (flip model) |
| Virtual monitor | Driver IddCx open source yang sudah ditandatangani (mis. Virtual Display Driver / SudoVDA). Jangan menulis driver sendiri. Verifikasi status signing & lisensi sebelum dipakai. |
| UI | Tauri 2 + React + TypeScript + Tailwind CSS (gunakan satuan rem/%/vw/vh, bukan px) |
| Konfigurasi | TOML di `%APPDATA%\Tautra\config.toml` |
| Logging | `tracing` + rotating file log di `%LOCALAPPDATA%\Tautra\logs` |

### 5.2 Proses

1. **tautra-service** (Windows Service, akun SYSTEM): injeksi input ke secure desktop (UAC, lock screen) dan window elevated, serta menjaga agent tetap hidup. Masuk di Fase 5. Sebelum itu, agent berjalan tanpa service.
2. **tautra-agent** (proses per user session, auto-start saat login): discovery, koneksi, input, clipboard, file, capture, encode, decode, render proxy window.
3. **tautra-ui** (Tauri): tray icon, halaman pengaturan, pairing, status. Berkomunikasi dengan agent via named pipe (`\\.\pipe\tautra-agent`) memakai JSON-RPC.

Agent dan UI terpisah supaya UI bisa ditutup tanpa memutus koneksi.

### 5.3 Struktur Repo

```
tautra/
├─ Cargo.toml                (workspace)
├─ crates/
│  ├─ tautra-proto/          (definisi pesan, serialisasi dengan postcard/bincode)
│  ├─ tautra-net/            (QUIC, discovery, pairing, reconnect)
│  ├─ tautra-input/          (hook, injeksi, edge detection, layout layar)
│  ├─ tautra-clipboard/      (teks, gambar, file)
│  ├─ tautra-transfer/       (transfer file & drag-drop)
│  ├─ tautra-capture/        (WGC capture per monitor & per window)
│  ├─ tautra-codec/          (encode/decode hardware)
│  ├─ tautra-render/         (D3D11 renderer, proxy window)
│  ├─ tautra-vdisplay/       (kontrol virtual monitor driver)
│  ├─ tautra-seamless/       (tracking window, z-order, hit-testing)
│  ├─ tautra-agent/          (binary agent)
│  └─ tautra-service/        (binary service)
├─ ui/                       (Tauri + React + Tailwind)
└─ docs/
```

## 6. Spesifikasi Fitur

### F1 — Discovery, Pairing, Auto Connect

- Setiap perangkat punya `device_id` (UUID) dan keypair yang dibuat saat pertama kali dijalankan, disimpan terenkripsi dengan DPAPI.
- Agent mengiklankan diri lewat mDNS `_tautra._udp.local` berisi `device_id`, nama perangkat, dan port QUIC.
- **Pairing pertama kali**: salah satu perangkat menampilkan PIN 6 digit, yang lain memasukkannya. Setelah berhasil, kedua perangkat menyimpan sertifikat masing-masing (certificate pinning). Perangkat yang belum di-pair ditolak.
- **Auto connect**: agent memantau perubahan jaringan (`NotifyIpInterfaceChange` / `NetworkListManager`). Saat jaringan berubah atau aktif, lakukan discovery. Jika ditemukan perangkat trusted, sambungkan otomatis.
- Reconnect dengan exponential backoff (0,5 dtk → maks 10 dtk) selama perangkat trusted masih terlihat di mDNS.
- **Mode idle**: jika tidak ada perangkat trusted ditemukan, discovery berjalan pasif (cek ulang setiap 30 detik atau saat event jaringan). Tidak ada hook input, capture, atau thread berat yang aktif. Target CPU idle <0,1%.
- Opsi: daftar SSID/jaringan yang diizinkan untuk auto connect (default: semua jaringan private).
- Heartbeat setiap 1 detik. Koneksi dianggap putus setelah 3 detik tanpa respons.

### F2 — Berbagi Keyboard & Mouse

- Di host input: low-level hook (`WH_MOUSE_LL`, `WH_KEYBOARD_LL`) di thread khusus prioritas tinggi, ditambah Raw Input untuk delta mouse relatif.
- **Layout layar**: konfigurasi posisi tiap monitor tiap perangkat dalam koordinat virtual bersama. Diatur via UI (drag kotak monitor, seperti Windows Display Settings). Mendukung multi-monitor dan DPI berbeda.
- **Edge crossing**: saat kursor menyentuh tepi yang berbatasan dengan perangkat lain, input berpindah. Kursor lokal disembunyikan/diparkir, event dikirim ke remote via QUIC datagram, lalu remote menginjeksi via `SendInput`.
- Posisi vertikal/horizontal dipetakan proporsional antar layar dengan resolusi berbeda.
- Opsi: penundaan edge (mis. harus "didorong" 150 ms) dan hanya pindah jika kursor ditahan, untuk mencegah pindah tidak sengaja.
- Mendukung semua tombol termasuk media keys, scroll horizontal, tombol mouse samping.
- Hotkey bawaan (bisa diubah):
  - `Ctrl+Alt+F1/F2`: lompat ke perangkat tertentu.
  - `Scroll Lock`: kunci kursor di perangkat saat ini.
  - `Ctrl+Alt+G`: toggle Gaming Mode.
- Target latensi input (LAN kabel): <2 ms p50, <5 ms p99. WiFi 5 GHz: <8 ms p99.
- Jika koneksi putus saat kursor ada di remote, kontrol otomatis kembali ke host input.

### F3 — Clipboard Bersama

- Pantau perubahan via `AddClipboardFormatListener`.
- Format: teks Unicode, HTML, RTF, gambar (PNG), dan daftar file (`CF_HDROP`).
- Sinkronisasi lazy: hanya metadata yang dikirim saat clipboard berubah. Konten penuh ditarik saat paste (delayed rendering) untuk konten >1 MB.
- Untuk file: paste di remote memicu transfer file (F4) lalu menempelkan file di lokasi tujuan.
- Opsi on/off per format dan batas ukuran.

### F4 — Transfer File & Drag and Drop

- Transfer via QUIC reliable stream, chunked, dengan progress, resume jika terputus, dan verifikasi hash (BLAKE3).
- Mendukung file besar (puluhan GB) dan folder (rekursif).
- **Drag and drop antar perangkat**:
  1. Deteksi drag OLE yang sedang berlangsung saat kursor melewati edge.
  2. Kirim metadata file ke remote.
  3. Di remote, mulai drag sintetis dengan `IDataObject` yang memakai `CFSTR_FILEDESCRIPTORW` + `CFSTR_FILECONTENTS` (virtual file, delayed rendering), sehingga file ditransfer saat di-drop di tujuan.
  4. Fallback MVP: file ditransfer ke folder `Downloads\Tautra` lalu Explorer dibuka di file tersebut.
- Notifikasi progress di tray untuk transfer >2 detik.

### F5 — Extended Display Mode (Layar Penuh)

> **Dibatalkan 2026-10-28** atas keputusan pengguna: fitur ini (dan F6 yang bergantung padanya) sempat diimplementasikan lalu dihapus seluruhnya karena menjeda perpindahan kursor KVM dan dinilai sumber bug. Spesifikasi di bawah disimpan sebagai arsip.

Fondasi sebelum seamless mode.
- Origin membuat virtual monitor (via driver IddCx) dengan resolusi & refresh rate yang sama dengan layar viewer. Posisinya di arrangement Windows sesuai layout F2.
- Origin men-capture virtual monitor tersebut (WGC per monitor) → encode hardware → kirim via QUIC datagram → viewer decode → tampil fullscreen di monitor viewer.
- Karena memakai virtual monitor, menyeret window ke tepi layar ditangani natively oleh Windows di origin. Drag otomatis smooth tanpa logika khusus.
- Input saat kursor berada di area virtual monitor diteruskan ke origin.
- Saat koneksi putus atau dimatikan: virtual monitor dicabut, Windows otomatis memindahkan semua window kembali ke monitor fisik.
- Pengaturan stream:
  - Codec: H.264 (default), HEVC, AV1 jika didukung kedua GPU.
  - FPS mengikuti refresh rate viewer (60/120/144).
  - Bitrate adaptif 20–100 Mbps berdasarkan RTT & packet loss.
  - Mode low-latency encoder: tanpa B-frame, intra refresh, ukuran GOP tak terbatas dengan keyframe on-demand saat packet loss.
- Transport video: fragmentasi frame ke datagram, nomor urut, FEC sederhana (opsional), drop frame lama alih-alih buffering.
- Target glass-to-glass latency: <20 ms p50 (LAN kabel, 60 fps), <35 ms p50 (WiFi 5 GHz).

### F6 — Seamless Window Mode (Fitur Utama)

> **Dibatalkan 2026-10-28** bersama F5 (lihat catatan F5).

Window dari origin tampil sebagai window individual di desktop viewer, bercampur dengan window lokal viewer.

- Virtual monitor di origin tetap dipakai sebagai "ruang" tempat window berada (sama seperti F5).
- Origin melacak window top-level yang berada di virtual monitor (`SetWinEventHook` untuk create/destroy/move/resize/foreground/minimize/z-order).
- Setiap window di-capture terpisah (WGC per window, `CreateForWindow`) dan di-stream sebagai stream terpisah (multiplexing per `window_id`).
- Viewer membuat proxy window untuk tiap window remote:
  - Borderless, posisi & ukuran sama persis (dikonversi dengan DPI).
  - Ikon & judul sama, muncul di taskbar viewer (dengan overlay ikon kecil penanda "dari Laptop").
  - Z-order dan fokus disinkronkan dua arah.
  - Mendukung shadow, rounded corner Windows 11, dan transparansi (alpha) jika memungkinkan.
- **Hit-testing input**: di viewer, jika kursor berada di atas proxy window, input diteruskan ke origin dengan koordinat yang dipetakan ke virtual monitor. Jika di atas window lokal, input diproses lokal. Pergantian harus instan tanpa kursor "melompat".
- **Drag antar layar**: saat pengguna menyeret window di origin melewati tepi ke virtual monitor, viewer langsung membuat proxy window yang mengikuti posisi drag secara real-time. Target: tanpa jeda yang terlihat.
- Window yang tidak bisa di-capture per window (fullscreen exclusive, protected content/DRM) fallback ke tampilan F5 untuk area tersebut, atau tampil sebagai placeholder dengan pesan.
- Menu popup, tooltip, dan dropdown dari aplikasi origin harus ikut tampil (tangkap juga window owned/popup).
- Arsitektur simetris: origin dan viewer bisa dibalik (window PC diseret ke laptop) dengan kode yang sama. Implementasi & uji arah laptop → PC lebih dulu.

### F7 — Gaming Mode

- Aktif via hotkey atau otomatis saat aplikasi fullscreen terdeteksi di host input (opsional, bisa dimatikan).
- Kursor terkunci di perangkat saat ini, edge crossing dimatikan.
- Hook input tetap pasif (langsung `CallNextHookEx` tanpa pemrosesan) atau dilepas sementara.
- Capture/stream yang tidak terlihat di-pause untuk membebaskan GPU.
- Jika game di-stream dari perangkat lain: mode mouse relatif (raw delta), prioritas latensi di atas kualitas.
- Catatan: beberapa anti-cheat memblokir input injeksi. Tampilkan peringatan di UI, jangan coba mengakali anti-cheat.

### F8 — Windows Service (Hak Akses Penuh)

- Service SYSTEM agar input bisa diinjeksi ke layar UAC, lock screen, dan window elevated.
- Agent per session berkomunikasi dengan service lewat named pipe yang diamankan ACL.
- Service me-restart agent jika crash.
- Hotkey khusus untuk mengirim `Ctrl+Alt+Del` (via `SendSAS`).

### F9 — UI (Tauri)

- **Tray icon**: status (Idle / Mencari / Tersambung / Error), menu cepat (putuskan, Gaming Mode, buka pengaturan, keluar).
- **Halaman**:
  1. Beranda: status koneksi, nama perangkat lawan, latensi & bitrate real-time.
  2. Perangkat: daftar perangkat trusted, pairing baru (tampil/masukkan PIN), hapus perangkat.
  3. Layout layar: drag-and-drop posisi monitor tiap perangkat.
  4. Input: hotkey, edge delay, kunci kursor.
  5. Clipboard & File: on/off format, batas ukuran, folder tujuan.
  6. Display & Streaming: codec, fps, bitrate maks, mode kualitas vs latensi, seamless on/off.
  7. Umum: auto-start, jaringan yang diizinkan, bahasa (Indonesia/English), log & diagnostik.
- Desain: modern, gelap/terang mengikuti tema Windows, efek Mica/Acrylic jika didukung Tauri. Tailwind dengan satuan rem/%/vw/vh, tanpa px, responsif di semua ukuran window dan DPI.
- Notifikasi toast Windows untuk connect/disconnect dan transfer file.

## 7. Protokol (Ringkas)

- Satu koneksi QUIC per pasangan perangkat.
- Reliable streams:
  - `control`: handshake, capability (GPU, codec, resolusi, DPI), layout, heartbeat, perintah.
  - `clipboard`, `file-transfer` (satu stream per transfer), `window-meta` (event window untuk seamless).
- Datagram:
  - `input` (mouse/keyboard, prioritas tertinggi, tanpa retransmit).
  - `video` (header: stream_id, frame_id, fragment_idx, fragment_count, keyframe flag, timestamp).
- Semua pesan versi-protokol diperiksa saat handshake. Jika versi berbeda, tampilkan pesan agar memperbarui aplikasi.
- Serialisasi biner (postcard) untuk performa.

## 8. Persyaratan Non-Fungsional

| Aspek | Target |
|---|---|
| Waktu auto connect setelah jaringan aktif | ≤5 detik |
| Latensi input (kabel) | <2 ms p50 |
| Glass-to-glass streaming (kabel, 60 fps) | <20 ms p50 |
| CPU idle (tidak tersambung) | <0,1% |
| CPU saat hanya KVM aktif | <1% |
| RAM agent idle | <30 MB |
| Stabilitas | Tidak ada window "hilang" saat disconnect, sleep, atau crash |
| Keamanan | Semua traffic terenkripsi (QUIC/TLS 1.3), hanya perangkat yang sudah di-pair |

## 9. Edge Cases yang Wajib Ditangani

- Laptop sleep/hibernate atau tutup lid saat tersambung → putus bersih, window kembali ke layar laptop saat bangun.
- WiFi putus di tengah drag window atau transfer file → drag dibatalkan dengan aman, transfer di-resume saat tersambung lagi.
- Resolusi/DPI/scaling berubah saat tersambung → layout & virtual monitor diperbarui.
- Monitor eksternal dicolok/dicabut di salah satu perangkat.
- Perangkat berganti IP (DHCP).
- Dua perangkat sama-sama mengira dirinya host input → aturan: host input ditentukan dari konfigurasi, bukan otomatis.
- GPU tidak mendukung hardware encoding → fallback software dengan peringatan performa.
- Driver virtual monitor belum terpasang → UI menampilkan panduan instalasi, fitur F1–F4 tetap berjalan.
- Secure desktop (UAC) muncul di remote saat service belum aktif → tampilkan pemberitahuan.

## 10. Rencana Pengerjaan (Fase)

Kerjakan berurutan. Jangan lanjut ke fase berikutnya sebelum kriteria penerimaan terpenuhi.

**Fase 0 — Fondasi**
- Workspace Rust, struktur crate, logging, konfigurasi, skeleton agent & UI Tauri, named pipe agent ↔ UI.
- Selesai jika: agent auto-start saat login, UI menampilkan status dari agent.

**Fase 1 — Discovery, Pairing, Auto Connect (F1)**
- Selesai jika: pairing PIN berhasil, dan setelah laptop restart/pindah jaringan lalu kembali, tersambung otomatis ≤5 detik tanpa interaksi.

**Fase 2 — KVM (F2)**
- Selesai jika: kursor & keyboard pindah mulus antar perangkat sesuai layout, hotkey berfungsi, latensi sesuai target.

**Fase 3 — Clipboard & File (F3, F4)**
- Selesai jika: copy-paste teks/gambar/file berfungsi, drag-drop file antar perangkat berfungsi (minimal versi fallback), file besar bisa di-resume.

**Fase 4 — Extended Display Mode (F5)** — *dibatalkan 2026-10-28*

**Fase 5 — Service & Gaming Mode (F7, F8)**
- Selesai jika: bisa mengontrol layar UAC & window elevated di remote, Gaming Mode mengunci kursor dan menurunkan overhead.

**Fase 6 — Seamless Window Mode (F6)** — *dibatalkan 2026-10-28*

**Fase 7 — Polish UI & Optimasi (F9)**
- Seluruh halaman UI lengkap, notifikasi, diagnostik latensi, optimasi bitrate adaptif, installer (MSI/NSIS via Tauri bundler).

## 11. Catatan untuk Implementasi

- Prioritaskan latensi di setiap keputusan: zero-copy GPU (capture → encode tanpa lewat CPU), thread khusus untuk input & video, hindari alokasi di hot path.
- Pelajari pendekatan dari project open source berikut sebagai referensi arsitektur (perhatikan lisensi GPL jika menyalin kode): Sunshine/Apollo (capture, encode, virtual display), Moonlight (decode & render latensi rendah), Deskflow/Input Leap (KVM), PowerToys Mouse Without Borders (clipboard & file).
- Semua teks UI mendukung Bahasa Indonesia dan English.
- Error harus terlihat jelas di UI dan log, bukan gagal diam-diam.
