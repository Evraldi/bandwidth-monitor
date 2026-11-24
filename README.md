# Bandwidth Monitor

Aplikasi desktop Windows untuk memantau bandwidth jaringan per proses. Frontend
Next.js + React, backend Rust lewat Tauri v2. Sumber data utama adalah ETW
(Event Tracing for Windows); kalau ETW tidak tersedia, aplikasi fallback ke
TCP EStats per koneksi.

## Prasyarat

- Windows 10/11
- Node.js 20+ dan npm
- Rust toolchain (via `rustup`) + WebView2 runtime
- Visual Studio Build Tools (MSVC) untuk `tauri build`

Untuk akurasi penuh (TCP + UDP termasuk koneksi yang langsung tutup), jalankan
sekali sebagai Administrator. Tanpa admin, aplikasi tetap jalan memakai
fallback TCP EStats yang kehilangan byte ekor koneksi yang sudah ditutup.

## Perintah

```bash
npm install
npm run tauri dev      # development
npm run tauri build    # MSI + NSIS di src-tauri/target/release/bundle/
```

## Test & quality gate

```bash
npm test               # unit test frontend (Node test runner, .mts)
npm run typecheck      # tsc --noEmit
npm run lint           # eslint
npm run test:rust      # cargo test
```

Test ETW butuh admin + session ETW yang bebas, jadi sengaja di-ignore oleh
`cargo test` biar CI tidak gagal:

```bash
cargo test etw_requires_elevation -- --ignored --nocapture
```

## Kontrak satuan

- `total_sent` / `total_recv`: total byte sejak aplikasi berjalan (kumulatif).
- `upload_bps` / `download_bps`: byte per detik, dihitung backend dari delta
  tiap siklus capture. Frontend hanya menampilkan, tidak menyimpan baseline.
- `formatBytes` memakai basis 1024 dan label biner (`KiB`/`MiB`/`GiB`).
- `spike_threshold_mib` adalah MiB per detik, bukan MiB per siklus capture.

## Data & retensi

Database SQLite ada di:

```
%APPDATA%\com.bandwidthmonitor.desktop\bandwidth.db
```

- `usage_snapshots`: snapshot mentah, dipangkas sesuai `retention_days`
  (default 7 hari, wall-clock).
- `daily_summary`: ringkasan harian, dipertahankan sampai 10 tahun supaya
  history bulanan tidak hilang. Ini sengaja berbeda dari `retention_days`.

Uninstaller NSIS punya checkbox **Delete app data** yang default-nya tidak
dicentang. Reinstall biasa tidak menghapus data. Backup dan script restore
tersedia di `backups/2026-11-24/`.

## Log

Log file ada di `%APPDATA%\com.bandwidthmonitor.desktop\app-logs\app.log`
(rotasi 5 file). Baris penting memakai format `kunci=nilai`, misalnya
`startup version=... admin=... retention_days=...` dan
`capture active=... running=... events_lost=... buffers_lost=...`.

## Catatan

Konfigurasi CI ada di `.github/workflows/ci.yml`, tapi belum jalan sampai
folder ini dijadikan repository Git dan di-push ke remote.
