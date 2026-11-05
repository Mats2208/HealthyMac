//! `HEALTHYMAC_DEMO=1`: invented but believable data, for designing full states and for showing the
//! app without showing someone's disk. It is badged in the UI and can never touch anything: in this
//! mode the app does not construct a `Cleaner`, send a signal, or write its config.

pub mod ai;
pub mod dev;
pub mod scan;
pub mod system;

use std::time::{Duration, SystemTime};

pub const HOME: &str = "/Users/ana";

pub fn days_ago(days: u64) -> Option<SystemTime> {
    Some(SystemTime::now() - Duration::from_secs(days * 86_400 + 1_800))
}

pub fn hours_ago(hours: u64) -> Option<SystemTime> {
    Some(SystemTime::now() - Duration::from_secs(hours * 3_600 + 120))
}

pub const GB: u64 = 1_000_000_000;
/// Memory is sold in these, so that is what a 64 GB Mac really has.
pub const GIB: u64 = 1024 * 1024 * 1024;
pub const MB: u64 = 1_000_000;
