//! Ringkasan log berbentuk `kunci=nilai` supaya mudah dicari di file
//! `app-logs/app.log` tanpa perlu parser khusus.

use log::LevelFilter;

pub fn level_for(debug_build: bool) -> LevelFilter {
    if debug_build {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    }
}

pub fn startup_summary(version: &str, admin: bool, retention_days: u32) -> String {
    format!("startup version={version} admin={admin} retention_days={retention_days}")
}

pub fn capture_summary(active: bool, running: bool, events_lost: u64, buffers_lost: u64) -> String {
    format!("capture active={active} running={running} events_lost={events_lost} buffers_lost={buffers_lost}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_release_tidak_debug() {
        assert_eq!(level_for(true), LevelFilter::Debug);
        assert_eq!(level_for(false), LevelFilter::Info);
    }

    #[test]
    fn ringkasan_log_bisa_dicari() {
        assert_eq!(
            startup_summary("0.1.0", true, 7),
            "startup version=0.1.0 admin=true retention_days=7"
        );
        assert_eq!(
            capture_summary(true, false, 3, 1),
            "capture active=true running=false events_lost=3 buffers_lost=1"
        );
    }
}
