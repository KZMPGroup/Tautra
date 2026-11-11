# Aturan animasi perpindahan kursor

Kode: `crates/tautra-input/src/crossing.rs`. Pemanggil: `hook.rs` (sisi yang memegang mouse fisik) dan `tautra-agent/src/kvm/session.rs` (sisi yang dikendalikan). Dokumen ini adalah aturan yang harus diikuti setiap gaya animasi, termasuk gaya baru yang ditambahkan nanti.

## Istilah

- **Masuk (Enter)**: kursor baru tiba di layar ini dari perangkat lain. Dimainkan oleh sisi yang kursornya kini bergerak di layar ini.
- **Keluar (Exit)**: kursor baru meninggalkan layar ini ke perangkat lain. Dimainkan oleh sisi yang kursornya baru saja pergi.
- **Titik jangkar (anchor)**: titik di tepi layar tempat kursor menyeberang. Diterima dari pesan `Enter`/`Leave` (bukan dibaca dari kursor fisik) lalu dijepit ke tepi yang dikirim `CrossEdge`. Jangkar **tidak pernah bergeser** selama animasi.
- **Koordinat (u, v)**: `u` = jarak ke dalam layar dari tepi (selalu positif ke dalam), `v` = pergeseran sepanjang tepi relatif jangkar. Semua rumus gaya ditulis dalam (u, v) sehingga sama untuk tepi kiri/kanan/atas/bawah; `render` yang memetakannya ke piksel.
- **Jalur (Path)**: urutan titik (u, v) dari jangkar `(0, 0)` yang dilalui kursor, dengan panjang busur kumulatif (`Path::pts`, `Path::cum`, maks 48 titik, titik baru hanya bila bergerak ≥ 1,5 px). Jalur **hanya bertambah**, tidak pernah digeser atau diputar, supaya arah animasi tidak melompat. `path.end()` adalah sasaran (`target`) untuk bentuk yang hanya butuh satu titik; `path.at(s)` dan `path.nearest(u, v, dari, sampai, jangkauan)` dipakai garis yang mengikuti jalur. Total panjang dibatasi `max_reach`.

## Aturan umum

1. **Kecil, transparan, di tepi.** Jendela animasi hanya seukuran animasi (`Spec.depth × maks(length, 2 × depth)` dikali skala DPI). Warna `ACCENT`, alpha maksimum `MAX_ALPHA`. Durasi 400–560 ms.
2. **Masuk dan keluar harus terasa berbeda** tetapi berpasangan: keluar adalah cermin dari masuk.
3. **Masuk mengikuti kursor fisik.** Tiap frame `follow_cursor` membaca `GetCursorPos`, menghitung (u, v) terhadap jangkar, dan menambahkannya ke jalur bila bergerak cukup jauh dari titik terakhir. Bila kursor keluar dari jendela animasi (`u > depth` atau `|v| > length/2`), satu titik terakhir ke arah itu ditambahkan (sampai `max_reach`) lalu mengikuti **berhenti**; jalur membeku. Kursor yang sangat cepat tetap menghasilkan arah dari frame pertama. Bila sampai 25% durasi kursor belum bergerak, `settle` menambahkan titik bawaan `(reach × skala, 0)` tegak lurus tepi dan berhenti mengikuti.
4. **Keluar mengikuti jejak kursor di perangkat lain** lewat `crossing::trail(dx, dy)` (delta piksel sejak titik masuk di layar seberang). Sumber delta:
   - sisi yang memegang mouse (`hook.rs`, mode Remote): dari posisi kursor virtual `remote.sent - remote.entry`, dipanggil langsung;
   - sisi yang dikendalikan lalu kursor pulang ke pemilik mouse: pemilik mouse mengirim `KvmMessage::Trail { dx, dy }` selama `TRAIL_WINDOW` (700 ms) setiap `TRAIL_INTERVAL` (6 ms) setelah `Leave`, dan `session.rs` meneruskannya ke `crossing::trail`.
   Delta dipetakan ke (u, v) layar keluar sebagai cermin: `u` = seberapa jauh kursor sudah melewati tepi, `v` = **negatif** pergeseran sepanjang tepi (kanan: `(dx, -dy)`, kiri: `(-dx, -dy)`, bawah: `(dy, -dx)`, atas: `(-dy, -dx)`), lalu ditambahkan ke jalur dengan aturan yang sama seperti masuk. Sebelum jejak pertama datang, garis keluar belum digambar (hanya pendar di tepi).
5. **Semua bentuk diposisikan relatif jalur**, bukan angka tetap: garis cahaya digambar sepanjang jalur (jarak piksel ke polyline, terang menurut panjang busur) dengan kepala, kilau pecahan, dan pendar di `path.at(head)`; pendar riak dan pita memakai `path.end()`. Bentuk yang simetris (cincin riak) boleh tetap berpusat di jangkar. Dilarang menggambar bentuk dari arah rata-rata atau arah terakhir saja, karena arahnya akan berputar saat kursor cepat.
6. **Murah.** Tanpa alokasi per frame, fungsi falloff polinomial (`bump`, `soft`), tempo mengikuti `DwmFlush`. Jangan menambah thread atau timer baru.
7. **Log trace** di build DEV: mulai (ukuran jendela, `following`), berhenti mengikuti (u, v, target), selesai (frame, ms, fps). Setiap `Err` dicatat minimal `warn`.
8. **Menambah gaya baru**: tambah varian di `CrossingStyle` (`tautra-common`), `Spec` (durasi, depth, length, reach, margin), dua cabang `Frame::new` (Exit dan Enter) yang memakai `target`/`dir`, cabang `intensity`, pilihan di `ui/src/pages/Input.tsx`, dan teks di 25 berkas i18n. Kode gaya baru ikut tersinkron lewat `KvmMessage::Prefs`.

## Gaya yang ada

| Gaya | Masuk | Keluar |
|------|-------|--------|
| Garis cahaya (default) | garis menyusuri jalur kursor dari jangkar, kepala + kilau 6 jari tepat di kursor, ekor menyusul lalu memudar | garis menyusuri cermin jalur kursor di perangkat seberang, memendek ke tepi; pendar di tepi menyusut |
| Riak air | pendar mengerut ke titik kursor, kilau di kursor | tiga cincin setengah lingkaran mengembang dari jangkar, pendar memudar |
| Geser & pendar | pita sejajar tepi bergeser sampai kedalaman kursor, pusatnya mengikuti kursor | pita bergeser ke tepi, pusat pita condong ke arah cermin jejak |
