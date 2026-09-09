//! The Recent place: what you saved lately, newest first, across your folders.
//!
//! The listing itself is not built here. Recent is a search with no query, so
//! it comes from the same provider the Everywhere scope uses — see
//! [`crate::search`], which walks the XDG user directories by modification
//! time on a worker and streams the result set in, taking the desktop's index
//! as a shortcut when there is one.
//!
//! What is here is everything that makes a listing ordered by time *read* as
//! one: the day buckets the grid puts headings between, and the sentinel path
//! that stands in for the directory a synthetic pane does not have.
//!
//! Bucketing lives beside the place rather than in the provider because it is
//! a presentation decision — "Yesterday" is a heading, not a query — and the
//! same entries would be bucketed differently by a window opened an hour after
//! midnight.

use std::time::SystemTime;

/// The sentinel `Column::path` for the recent listing.
///
/// Not a path anyone can navigate to. It exists so the column keeps its
/// `PathBuf` field rather than making every reader handle an `Option`, and so
/// a stray navigation to it fails to read a directory rather than reading the
/// wrong one. Everything that shows the location to a user checks the
/// browser's `recent` flag first and never renders this.
pub const SENTINEL: &str = "/dev/null/otto-recent";

/// Is this the recent listing's sentinel?
pub fn is_sentinel(path: &std::path::Path) -> bool {
    path == std::path::Path::new(SENTINEL)
}

/// Which day-bucket a modification time falls in.
///
/// Ordered as the grid shows them, newest first, so the discriminant order is
/// the display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bucket {
    Today,
    Yesterday,
    ThisWeek,
    ThisMonth,
    Earlier,
}

impl Bucket {
    /// The heading drawn above this bucket's run of tiles.
    pub fn label(self) -> &'static str {
        match self {
            Bucket::Today => otto_kit::t!("files-recent-today"),
            Bucket::Yesterday => otto_kit::t!("files-recent-yesterday"),
            Bucket::ThisWeek => otto_kit::t!("files-recent-this-week"),
            Bucket::ThisMonth => otto_kit::t!("files-recent-this-month"),
            Bucket::Earlier => otto_kit::t!("files-recent-earlier"),
        }
    }
}

/// Seconds since the Unix epoch, or `None` for a time before it.
fn epoch_secs(t: SystemTime) -> Option<i64> {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

const DAY: i64 = 86_400;

/// The start of the local day containing `secs`.
///
/// Buckets are cut at **local midnight**, not at `now - 24h`: "Yesterday" has
/// to mean yesterday, or a file saved at nine last night lands under Today at
/// eight this morning and the heading is a lie. `localtime_r` gives the offset
/// from UTC in force at that moment, which is what makes this right across a
/// daylight-saving change rather than only most of the year.
fn local_midnight(secs: i64) -> i64 {
    let offset = local_utc_offset(secs);
    let local = secs + offset;
    local - local.rem_euclid(DAY) - offset
}

/// The UTC offset in seconds in force locally at `secs`.
fn local_utc_offset(secs: i64) -> i64 {
    // SAFETY: `tm` is fully written by `localtime_r` before it is read, and
    // the reentrant form is used so this is safe off the main thread.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let t = secs as libc::time_t;
        if libc::localtime_r(&t, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff as i64
    }
}

/// Which bucket `modified` falls in, relative to `now`.
///
/// An entry with no modification time at all sorts into `Earlier`: it is the
/// bucket that promises the least, and a file whose time could not be read
/// must not claim to be from today.
pub fn bucket_of(modified: Option<SystemTime>, now: SystemTime) -> Bucket {
    let (Some(then), Some(now)) = (modified.and_then(epoch_secs), epoch_secs(now)) else {
        return Bucket::Earlier;
    };
    let today = local_midnight(now);
    // A clock that has gone backwards, or a file stamped in the future: it is
    // the most recent thing there is, which is Today.
    if then >= today {
        return Bucket::Today;
    }
    if then >= today - DAY {
        return Bucket::Yesterday;
    }
    if then >= today - 7 * DAY {
        return Bucket::ThisWeek;
    }
    if then >= today - 30 * DAY {
        return Bucket::ThisMonth;
    }
    Bucket::Earlier
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ago(secs: i64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(secs as u64)
    }

    #[test]
    fn a_file_saved_a_minute_ago_is_from_today() {
        assert_eq!(bucket_of(Some(ago(60)), SystemTime::now()), Bucket::Today);
    }

    #[test]
    fn the_buckets_are_cut_at_local_midnight_not_at_a_rolling_day() {
        let now = SystemTime::now();
        let now_secs = epoch_secs(now).unwrap();
        let midnight = local_midnight(now_secs);
        // One second either side of this morning's midnight: the earlier of
        // the two is yesterday however early in the day it is now, which a
        // rolling `now - 24h` window would get wrong for most of the day.
        let just_after = SystemTime::UNIX_EPOCH + Duration::from_secs(midnight as u64);
        let just_before = SystemTime::UNIX_EPOCH + Duration::from_secs(midnight as u64 - 1);
        assert_eq!(bucket_of(Some(just_after), now), Bucket::Today);
        assert_eq!(bucket_of(Some(just_before), now), Bucket::Yesterday);
    }

    #[test]
    fn a_file_with_no_time_claims_nothing() {
        assert_eq!(bucket_of(None, SystemTime::now()), Bucket::Earlier);
    }
}
