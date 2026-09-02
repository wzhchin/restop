use anyhow::Context;
use chin_tools::AResult;
use chrono::{DateTime, NaiveDateTime};
use once_cell::sync::Lazy;

use super::process_data::unix_as_millis;

pub fn human_time() -> String {
    let fmt = "%Y-%m-%d %H:%M:%S";

    chrono::Local::now().format(fmt).to_string()
}

static BOOT_TIMESTAMP: Lazy<Option<i64>> = Lazy::new(|| {
    let unix_timestamp = (unix_as_millis() / 1000) as i64;
    std::fs::read_to_string("/proc/uptime")
        .context("unable to read /proc/uptime")
        .and_then(|procfs| {
            procfs
                .split(' ')
                .next()
                .map(str::to_string)
                .context("unable to split /proc/uptime")
        })
        .and_then(|uptime_str| {
            uptime_str
                .parse::<f64>()
                .context("unable to parse /proc/uptime")
        })
        .map(|uptime_secs| unix_timestamp - uptime_secs as i64)
        .ok()
});

pub fn boot_time() -> AResult<NaiveDateTime> {
    BOOT_TIMESTAMP
        .context("couldn't get boot timestamp")
        .and_then(|timestamp| {
            DateTime::from_timestamp(timestamp, 0)
                .map(|e| e.naive_local())
                .context("unable to read")
        })
}
