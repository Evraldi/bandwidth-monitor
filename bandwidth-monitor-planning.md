# Planning: Bandwidth Monitor (Self-Hosted Data Usage Tracker)

## Overview
Aplikasi desktop Windows yang memonitor penggunaan bandwidth internet secara real-time per aplikasi/proses, menyimpan history harian/bulanan ke database lokal, dan menampilkan grafik serta ringkasan pemakaian. Tidak ada cloud, tidak ada langganan — semua lokal di PC sendiri.

## Tech Stack
- **Backend/Shell**: Tauri v2 (Rust)
- **Frontend**: Next.js (App Router, TypeScript, Tailwind CSS), static export
- **Data capture (Windows)**: ETW (Event Tracing for Windows) via dua pendekatan yang dikombinasi:
  - **`ferrisetw`** (crate) — Rust-native ETW consumer, subscribe ke provider `Microsoft-Windows-TCPIP` buat tangkap event network per koneksi
  - **`windows` crate** + `GetPerTcpConnectionEStats` / `GetPerTcp6ConnectionEStats` — ambil byte sent/received tiap koneksi TCP (IPv4 + IPv6), di-correlate ke PID via `GetExtendedTcpTable`
  - **`sysinfo`** — resolve PID ke nama proses/exe
- **Storage**: SQLite lokal via crate `rusqlite` — cocok buat time-series data yang numpuk per hari
- **Chart**: `recharts` di frontend buat grafik harian/bulanan

## Catatan Penting: Privilege
`GetPerTcpConnectionEStats` dan ETW session capture **butuh run as Administrator**. Tauri bisa dikonfigurasi minta UAC prompt otomatis saat app di-launch lewat `requestedExecutionLevel` di `tauri.conf.json`:
```json
"windows": {
  "requestedExecutionLevel": "requireAdministrator"
}
```

## Setup / Install Commands

Prasyarat:
```bash
# Semua udah ada dari setup port-monitor / dev-launcher sebelumnya
rustc --version     # 1.97.1+
cargo --version
cargo tauri --version
node --version
```

Bikin project baru:
```bash
cd D:\projek
mkdir bandwidth-monitor
cd bandwidth-monitor

# Init Next.js
npx create-next-app@latest .
# TypeScript Yes, ESLint Yes, React Compiler No
# Tailwind Yes, src/ dir Yes, App Router Yes, import alias @/*

# Init Tauri
cargo tauri init
# App name: bandwidth-monitor
# Window title: Bandwidth Monitor
# Web assets: ../out
# Dev server URL: http://localhost:3000
# Dev command: npm run dev
# Build command: npm run build
```

Next.js config (`next.config.ts`):
```typescript
const nextConfig: NextConfig = {
  output: "export",
  images: { unoptimized: true },
};
```

Dependency Rust (tambahkan ke `src-tauri/Cargo.toml`):
```toml
[dependencies]
sysinfo = "0.32"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
rusqlite = { version = "0.31", features = ["bundled"] }
ferrisetw = "0.5"

[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
  "Win32_NetworkManagement_IpHelper",
  "Win32_Networking_WinSock",
  "Win32_Foundation",
] }
```

Frontend:
```bash
npm install @tauri-apps/api recharts
npm install -D @types/recharts
```

## Cara Kerja ETW + EStats (Pendekatan Teknis)

### Flow data capture:
```
ETW Session (ferrisetw)
  └─ subscribe ke "Microsoft-Windows-TCPIP"
      └─ event: koneksi baru dibuat → catat {PID, local_port, remote_ip}

GetExtendedTcpTable (windows crate, polling ~2 detik)
  └─ list semua koneksi TCP aktif + PID

GetPerTcpConnectionEStats per koneksi
  └─ DataBytesIn + DataBytesOut per koneksi

Correlate PID → nama proses (sysinfo)
  └─ hitung delta bytes dari polling sebelumnya → dapat KB/s real-time

Simpan ke SQLite
  └─ tabel: process_usage (timestamp, exe_name, pid, bytes_sent, bytes_recv)
  └─ query: agregat per jam/hari/bulan
```

### Kenapa kombinasi ETW + EStats:
- ETW bagus buat deteksi koneksi baru dan event-based (push)
- `GetPerTcpConnectionEStats` bagus buat akurasi bytes per koneksi (pull)
- Kombinasi keduanya = data real-time yang akurat dan low-overhead

## Skema Database SQLite

```sql
-- usage per sesi/snapshot (raw data)
CREATE TABLE usage_snapshots (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  timestamp INTEGER NOT NULL,       -- unix timestamp
  exe_name TEXT NOT NULL,
  pid INTEGER NOT NULL,
  bytes_sent INTEGER NOT NULL,
  bytes_recv INTEGER NOT NULL
);

-- agregat harian (diisi via scheduled summarizer tiap tengah malam / app close)
CREATE TABLE daily_summary (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  date TEXT NOT NULL,               -- format: YYYY-MM-DD
  exe_name TEXT NOT NULL,
  total_sent INTEGER NOT NULL,
  total_recv INTEGER NOT NULL,
  UNIQUE(date, exe_name)
);

CREATE INDEX idx_snapshots_ts ON usage_snapshots(timestamp);
CREATE INDEX idx_daily_date ON daily_summary(date);
```

## Tauri Commands (Rust)

```rust
#[tauri::command]
async fn get_realtime_usage() -> Vec<ProcessUsage>
// return: list proses + current KB/s upload & download, total sesi ini

#[tauri::command]
fn get_daily_summary(date: String) -> Vec<DailySummary>
// return: list proses + total sent/recv untuk tanggal tertentu

#[tauri::command]
fn get_monthly_summary(year: u32, month: u32) -> Vec<MonthlySummary>
// return: total per proses untuk satu bulan penuh

#[tauri::command]
fn get_history_chart(exe_name: String, days: u32) -> Vec<DailyDataPoint>
// return: data per hari N hari terakhir untuk satu proses (buat grafik)
```

## Fitur Frontend (UI)

### Tab 1: Real-time
- Tabel proses aktif yang lagi transfer data:
  `Nama App | Upload KB/s | Download KB/s | Total sesi`
- Auto-refresh tiap 2 detik via `invoke("get_realtime_usage")`
- Sort by kolom (klik header), filter by nama proses
- Color indicator: hijau (normal), kuning (>1MB/s), merah (>10MB/s)

### Tab 2: History (Harian)
- Date picker buat pilih tanggal
- Bar chart horizontal: top 10 app paling banyak makan data hari itu
- Detail tabel di bawahnya

### Tab 3: Monthly
- Pilih bulan/tahun
- Grafik line chart: total data usage per hari dalam satu bulan
- Breakdown per app di bawahnya
- Summary: total bulan ini upload + download

### Alert (Optional, v2)
- Notif Windows kalau ada proses yang tiba-tiba spike (>100MB dalam 1 menit)
- Pake `tauri-plugin-notification`

## Urutan Pengerjaan (Disarankan)
1. Setup project (command install di atas)
2. Setup SQLite schema — buat helper module buat init DB dan basic CRUD
3. Backend: implement `GetExtendedTcpTable` + `GetPerTcpConnectionEStats` buat polling bytes per koneksi → hasilnya delta bytes per proses tiap 2 detik
4. Backend: simpan snapshot ke SQLite tiap polling
5. Backend: implement `get_realtime_usage` command
6. Frontend: Tab Real-time — tabel proses + auto-refresh
7. Backend: implement summarizer (agregat ke `daily_summary`) + query commands
8. Frontend: Tab History & Monthly — chart + tabel
9. Integrasi ETW via `ferrisetw` buat deteksi koneksi baru (opsional, bisa skip dulu buat MVP)
10. UAC config di `tauri.conf.json` biar auto-minta admin saat launch
11. Test: `cargo tauri dev` → `cargo tauri build`

## Catatan Tambahan
- UDP traffic lebih susah di-track akurat pake EStats (EStats hanya support TCP) — buat MVP, fokus TCP dulu. UDP bisa ditambah lewat ETW provider terpisah di v2.
- Karena butuh admin privilege, app ini nggak bisa jalan tanpa "Run as Administrator" atau UAC config — inform user saat pertama launch.
- `rusqlite` dengan feature `bundled` artinya SQLite di-compile langsung ke dalam binary Rust — nggak perlu install SQLite terpisah di PC target.
- History data bisa tumbuh besar kalau dibiarkan — tambahkan auto-cleanup (hapus snapshot > 7 hari, keep hanya daily_summary) supaya database tetap ringan.
