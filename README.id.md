<p align="center">
  <img src="ui/public/logo.svg" alt="Tautra" width="88" />
</p>

<h1 align="center">Tautra</h1>

<p align="center">
  Satu keyboard dan mouse, satu clipboard, dan seret file antar dua PC Windows.
</p>

<p align="center">
  <a href="README.md">English</a> · Bahasa Indonesia
</p>

---

Tautra adalah KVM software untuk Windows. Tautra menyatukan dua perangkat Windows di jaringan yang sama, misalnya PC dan laptop, sehingga terasa seperti satu meja kerja. Dorong kursor ke tepi layar, kursor pindah ke perangkat lain, dan keyboard, clipboard, serta file yang diseret ikut pindah.

Semua lalu lintas berjalan langsung antar perangkat (peer to peer) lewat QUIC dan TLS. Tidak ada server cloud, akun, atau telemetri.

## Fitur

**Berbagi keyboard dan mouse**
- Kursor pindah lewat tepi layar yang bersebelahan, dengan editor layout visual yang bisa disesuaikan dengan posisi meja.
- Mendukung keyboard, semua tombol mouse, scroll vertikal dan horizontal, tombol samping, dan tombol media.
- Hotkey untuk pindah perangkat dan mengunci kursor. Setiap hotkey bisa dikembalikan ke bawaan.
- Gaming Mode: mengunci kursor di perangkat ini. Mode ini menyala otomatis saat ada aplikasi fullscreen.
- Layanan Windows (opsional) supaya kursor bisa mengendalikan UAC, layar kunci, dan jendela administrator. Jika layanan terpasang dan "Mulai saat masuk Windows" menyala, PC juga bisa dikontrol dari layar masuk Windows sejak baru dinyalakan.

**Clipboard dan file**
- Sinkronisasi clipboard untuk teks, HTML, RTF, gambar, dan file.
- Konten clipboard besar baru dikirim saat di-paste.
- Seret dan lepas file antar perangkat, lengkap dengan progres, tombol batal, dan "tampilkan di folder".

**Koneksi**
- Pairing dengan PIN 6 digit. Setelah itu sertifikat perangkat dikunci, jadi perangkat lain tidak bisa ikut tersambung.
- Pencarian otomatis lewat mDNS, atau pairing manual lewat alamat IP, misalnya lewat Tailscale atau VPN.
- Tersambung lagi otomatis setelah sleep, ganti jaringan, atau ganti IP.
- Opsi "Hanya jaringan yang diizinkan" untuk membatasi jaringan dan adapter yang boleh dipakai.
- Lalu lintas Tautra diberi prioritas tinggi lewat QoS (DSCP), dan agent berjalan dengan prioritas proses lebih tinggi.

**Perawatan**
- Auto update antar perangkat: perangkat dengan build lebih lama otomatis mengambil build terbaru dari perangkat lain.
- Ukuran log dibatasi dan file log lama dibersihkan otomatis.
- Tampilan, menu tray, dan notifikasi tersedia dalam 25 bahasa. Tautra otomatis memakai bahasa tampilan Windows (bahasa Inggris jika bahasa itu belum tersedia), dan bahasa bisa diganti serta dicari di halaman **Umum**.
- Tema terang dan gelap, serta ikon tray.

## Kebutuhan

- Windows 10 versi 2004 (build 19041) atau lebih baru, atau Windows 11. Edisi Home, Pro, Enterprise, dan LTSC didukung.
- x64 atau ARM64.
- WebView2 Runtime, yang sudah ada di Windows 11 dan Windows 10 terbaru.
- Kedua perangkat harus bisa saling menjangkau: satu LAN atau Wi-Fi, atau lewat VPN seperti Tailscale.

## Cara pakai

1. Unduh rilis terbaru dari [halaman Releases](https://github.com/KZMPGroup/Tautra/releases) di kedua perangkat. Pilih installer (`Tautra_x.y.z_x64-setup.exe`) atau zip portable.
2. Jalankan installer, atau ekstrak zip sehingga `tautra-ui.exe` dan `tautra-agent.exe` ada di satu folder. Lalu jalankan Tautra.
3. Saat Windows Firewall bertanya, izinkan Tautra di jaringan kamu.
4. Di perangkat pertama, buka **Perangkat → Tampilkan PIN**.
5. Di perangkat kedua, buka **Perangkat → Masukkan PIN**, pilih perangkat pertama atau ketik IP-nya, lalu masukkan PIN.
6. Buka **Layout layar** dan atur posisi perangkat sesuai meja.
7. Opsional: di halaman **Input**, pasang layanan Tautra di kedua perangkat supaya kursor bisa mengendalikan UAC dan layar kunci. Ini butuh hak administrator sekali saja.

### Hotkey bawaan

| Aksi | Hotkey |
| --- | --- |
| Kembalikan kursor ke perangkat ini | `Ctrl+Alt+F1` |
| Pindah ke perangkat lain | `Ctrl+Alt+F2` |
| Kunci kursor di perangkat saat ini | `Scroll Lock` |
| Gaming Mode | `Ctrl+Alt+G` |

Semua hotkey bisa diubah di halaman **Input**.

### Jaringan

| Port | Protokol | Kegunaan |
| --- | --- | --- |
| 47800 (bawaan) | UDP | Koneksi QUIC antar perangkat |
| 5353 | UDP | Pencarian mDNS |

### Lokasi file

| Isi | Lokasi |
| --- | --- |
| Pengaturan dan layout layar | `%APPDATA%\Tautra` |
| Perangkat tepercaya, identitas, dan log | `%LOCALAPPDATA%\Tautra` |
| Layanan (jika dipasang) | `%ProgramData%\Tautra\service` |

## Build dari source

Yang dibutuhkan:

- Rust 1.85 atau lebih baru, dengan toolchain MSVC (`x86_64-pc-windows-msvc`).
- Visual Studio Build Tools dengan workload "Desktop development with C++".
- Node.js LTS.

```powershell
git clone https://github.com/KZMPGroup/Tautra.git
cd Tautra\ui
npm install
npm run tauri build -- --no-bundle
```

Hasilnya `tautra-ui.exe` dan `tautra-agent.exe` di `target\release`. Salin keduanya ke satu folder.

Untuk membuat installer, jalankan `npm run tauri build` tanpa `--no-bundle`. Hasilnya ada di `target\release\bundle\nsis`. Untuk pengembangan, jalankan `npm run tauri dev` di folder `ui`.

## Arsitektur

Tautra terdiri dari dua proses:

- **`tautra-agent`** berjalan di background dan mengerjakan semuanya: jaringan, hook input, clipboard, transfer file, dan update.
- **`tautra-ui`** adalah jendela Tauri + React dan ikon tray.

Keduanya berkomunikasi lewat JSON-RPC di named pipe. Menutup jendela tidak menghentikan berbagi. Rincian tiap crate ada di [README.md](README.md#architecture).

Spesifikasi awal ada di [docs/PRD.md](docs/PRD.md), dan catatan pengembangan ada di [docs/PROGRESS.md](docs/PROGRESS.md).

## Lisensi

Hak cipta (C) 2026 KZMP Group

Tautra adalah perangkat lunak bebas di bawah [GNU General Public License v3.0](LICENSE). Kamu boleh memakainya secara gratis, baik pribadi maupun di kantor. Kamu juga boleh mempelajari dan mengubah source code-nya.

Siapa pun yang mendistribusikan Tautra atau versi modifikasinya wajib membuka seluruh source code-nya dengan lisensi GPL-3.0 yang sama dan tetap mencantumkan hak cipta. Versi tertutup atau proprietary tidak diizinkan.

Perbaikan dan fitur baru sebaiknya dikirim ke repo ini lewat pull request, supaya semua pengguna mendapatkannya di satu tempat. Mohon jangan merilis salinan terpisah dengan nama lain.

## Kontribusi

Issue dan pull request sangat diterima. Lihat [CONTRIBUTING.md](CONTRIBUTING.md).
