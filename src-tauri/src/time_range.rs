//! Rentang waktu harian yang aman terhadap DST.
//!
//! Snapshot disimpan sebagai epoch detik (UTC), tapi "hari" adalah konsep
//! lokal. Di hari transisi, jam 00:00 bisa ambigu (mundur) atau tidak ada
//! (maju), jadi `.single()` tidak boleh dipakai. Di sini awal hari memakai
//! kandidat paling awal dan akhir hari (eksklusif) memakai kandidat paling
//! akhir, supaya tidak ada snapshot yang terlewat.

use chrono::{Duration as ChronoDuration, Local, LocalResult, NaiveDate, NaiveDateTime};

/// `resolve` mengembalikan kandidat epoch detik untuk suatu waktu lokal naive.
/// Dua kandidat berarti waktu ambigu; kosong berarti waktu itu tidak ada.
/// Urutan kandidat harus dari yang paling awal ke yang paling akhir.
fn resolve_one<F>(naive: NaiveDateTime, resolve: &F, prefer_latest: bool) -> Option<i64>
where
    F: Fn(NaiveDateTime) -> Option<Vec<i64>>,
{
    // Kalau tengah malam tidak ada (DST maju), geser maju sampai ketemu waktu
    // lokal yang valid. Maksimal 24 jam, jadi tidak mungkin loop tanpa akhir.
    for hours in 0..=24 {
        let shifted = naive.checked_add_signed(ChronoDuration::hours(hours))?;
        let Some(candidates) = resolve(shifted) else {
            continue;
        };
        let picked = if prefer_latest {
            candidates.last().copied()
        } else {
            candidates.first().copied()
        };
        if picked.is_some() {
            return picked;
        }
    }
    None
}

/// Versi murni supaya kasus DST bisa di-test tanpa mengubah TZ mesin.
pub fn day_bounds_with<F>(
    start: NaiveDateTime,
    end: NaiveDateTime,
    resolve: F,
) -> Option<(i64, i64)>
where
    F: Fn(NaiveDateTime) -> Option<Vec<i64>>,
{
    let start_ts = resolve_one(start, &resolve, false)?;
    let end_ts = resolve_one(end, &resolve, true)?;
    (start_ts < end_ts).then_some((start_ts, end_ts))
}

/// Rentang `[00:00 hari ini, 00:00 hari berikutnya)` dalam epoch detik.
/// Memakai interval setengah terbuka supaya detik 23:59:59 tidak perlu ditebak.
pub fn local_day_bounds(date: NaiveDate) -> Option<(i64, i64)> {
    let start = date.and_hms_opt(0, 0, 0)?;
    let end = date
        .checked_add_signed(ChronoDuration::days(1))?
        .and_hms_opt(0, 0, 0)?;

    day_bounds_with(start, end, |naive| match naive.and_local_timezone(Local) {
        LocalResult::None => None,
        mapped => {
            let early = mapped.earliest()?.timestamp();
            let late = mapped.latest()?.timestamp();
            Some(if early == late {
                vec![early]
            } else {
                vec![early, late]
            })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 24).unwrap()
    }

    #[test]
    fn batas_hari_utc_setengah_terbuka() {
        let d = date();
        let start = d.and_hms_opt(0, 0, 0).unwrap();
        let end = d.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap();
        let bounds = day_bounds_with(start, end, |n| Some(vec![n.and_utc().timestamp()])).unwrap();
        assert_eq!(bounds.1 - bounds.0, 24 * 3600);
    }

    #[test]
    fn tengah_malam_ambigu_tidak_bikin_gagal() {
        let d = date();
        let start = d.and_hms_opt(0, 0, 0).unwrap();
        let end = d.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap();
        let start_utc = start.and_utc().timestamp();
        let end_utc = end.and_utc().timestamp();

        // Dua kandidat: offset sebelum/sesudah jam mundur.
        let bounds = day_bounds_with(start, end, |n| {
            let base = n.and_utc().timestamp();
            if n.hour() == 0 {
                Some(vec![base, base + 3600])
            } else {
                Some(vec![base])
            }
        })
        .unwrap();

        assert_eq!(bounds.0, start_utc, "awal hari ambil kandidat paling awal");
        assert_eq!(
            bounds.1,
            end_utc + 3600,
            "akhir hari ambil kandidat paling akhir"
        );
    }

    #[test]
    fn tengah_malam_yang_tidak_ada_digeser_maju() {
        let d = date();
        let start = d.and_hms_opt(0, 0, 0).unwrap();
        let end = d.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap();

        let bounds = day_bounds_with(start, end, |n| {
            if n.hour() == 0 {
                None
            } else {
                Some(vec![n.and_utc().timestamp()])
            }
        })
        .unwrap();

        assert_eq!(bounds.0 - start.and_utc().timestamp(), 3600);
        assert_eq!(bounds.1 - end.and_utc().timestamp(), 3600);
    }

    #[test]
    fn local_day_bounds_tidak_panic_di_tz_mesin() {
        let (start, end) = local_day_bounds(date()).unwrap();
        assert!(end > start);
    }
}
