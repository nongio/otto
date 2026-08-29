//! Listings: directories, and archives.
//!
//! **Listing an archive needs no decompression.** A zip's central directory
//! carries every entry's name, size and date in plain form at the end of the
//! file, and a tar is 512-byte headers. That is why archives are a v1 type
//! despite Otto having no inflate implementation — this is a parse, not an
//! extraction. A *compressed* tar cannot be listed without inflating and is
//! deliberately shown as a plain file card instead.

use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::AsRawFd;

use otto_kit::filetype;

use crate::payload;
use crate::payload::{PreviewPayload, Row};

use super::{human_size, read_capped, Request};

/// A listing is a look at what is inside, not a file manager. Enough rows to
/// answer "what is this", few enough that the parent never lays out a
/// hundred-thousand-row table.
const MAX_ROWS: usize = 2_000;

// ---------------------------------------------------------------------------
// Directories
// ---------------------------------------------------------------------------

/// The worker holds a descriptor, not a path — deliberately, so no name can be
/// substituted underneath it. `fdopendir` is how a directory is read from one.
pub fn directory(file: &mut File, _request: &Request) -> PreviewPayload {
    let mut rows = Vec::new();
    let mut total = 0u64;
    let mut truncated = false;

    // SAFETY: `fdopendir` takes ownership of the descriptor it is given, so it
    // gets a duplicate — the caller's `File` still owns the original and will
    // close it. Every pointer below is checked before use, and `readdir` is
    // called on one directory stream from one thread.
    unsafe {
        let duplicate = libc::dup(file.as_raw_fd());
        if duplicate < 0 {
            return payload::unavailable(otto_kit::t_owned!("quickview-error-read-folder"));
        }
        let dir = libc::fdopendir(duplicate);
        if dir.is_null() {
            libc::close(duplicate);
            return payload::unavailable(otto_kit::t_owned!("quickview-error-read-folder"));
        }

        loop {
            let entry = libc::readdir(dir);
            if entry.is_null() {
                break;
            }
            let name = std::ffi::CStr::from_ptr((*entry).d_name.as_ptr())
                .to_string_lossy()
                .into_owned();
            if name == "." || name == ".." {
                continue;
            }
            // Hidden entries are counted but not listed: a preview of a source
            // checkout should not be mostly dotfiles.
            if name.starts_with('.') {
                total += 1;
                continue;
            }
            total += 1;
            if rows.len() >= MAX_ROWS {
                truncated = true;
                continue;
            }
            let is_dir = (*entry).d_type == libc::DT_DIR;
            let icon = if is_dir {
                "folder".to_string()
            } else {
                filetype::kind_for_name(&name).generic_icon().to_string()
            };
            rows.push(Row {
                name,
                size: 0,
                mtime: 0,
                icon,
                is_dir,
            });
        }
        libc::closedir(dir);
    }

    // Folders first, then by name, case-insensitively — the order a person
    // expects rather than the order the filesystem happens to return.
    rows.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    PreviewPayload::Rows {
        rows,
        truncated,
        summary: if total == 0 {
            otto_kit::t_owned!("quickview-empty-folder")
        } else {
            otto_kit::t_owned!("quickview-item-count", count = total as f64)
        },
    }
}

// ---------------------------------------------------------------------------
// Zip
// ---------------------------------------------------------------------------

/// How far back from the end of the file the end-of-central-directory record
/// may start: 22 bytes of record plus a comment field of up to 64 KB.
const EOCD_SEARCH: u64 = 22 + 0xFFFF;
const EOCD_SIGNATURE: u32 = 0x0605_4B50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4B50;

pub fn zip(file: &mut File, metadata: &Metadata, request: &Request) -> PreviewPayload {
    match zip_rows(file, metadata) {
        Some((rows, truncated, total)) => PreviewPayload::Rows {
            rows,
            truncated,
            summary: otto_kit::t_owned!(
                "quickview-archive-summary",
                items = otto_kit::t_owned!("quickview-item-count", count = total as f64),
                size = human_size(metadata.len())
            ),
        },
        // A zip we cannot walk is still a file we can describe.
        None => super::media::generic(metadata, request, "application/zip"),
    }
}

/// Walk the central directory. Returns `None` if the file is not a zip we can
/// read — never a panic, and never a partial listing presented as complete.
fn zip_rows(file: &mut File, metadata: &Metadata) -> Option<(Vec<Row>, bool, usize)> {
    let length = metadata.len();
    let tail_len = EOCD_SEARCH.min(length);
    file.seek(SeekFrom::End(-(tail_len as i64))).ok()?;
    let mut tail = vec![0u8; tail_len as usize];
    file.read_exact(&mut tail).ok()?;

    // The record is at the end, so scan backwards: a zip containing a zip would
    // otherwise match the inner one's signature first.
    let eocd = (0..=tail.len().checked_sub(22)?)
        .rev()
        .find(|at| read_u32(&tail, *at) == Some(EOCD_SIGNATURE))?;

    let entry_count = read_u16(&tail, eocd + 10)? as usize;
    let directory_offset = read_u32(&tail, eocd + 16)? as u64;
    let directory_size = read_u32(&tail, eocd + 12)? as usize;
    if directory_offset >= length {
        return None;
    }

    file.seek(SeekFrom::Start(directory_offset)).ok()?;
    // The central directory is names and numbers; it is not a decompression.
    let mut directory = vec![0u8; directory_size.min(32 * 1024 * 1024)];
    let read = file.read(&mut directory).ok()?;
    directory.truncate(read);

    let mut rows = Vec::new();
    let mut truncated = false;
    let mut at = 0usize;
    let mut seen = 0usize;

    while seen < entry_count && at + 46 <= directory.len() {
        if read_u32(&directory, at) != Some(CENTRAL_SIGNATURE) {
            break;
        }
        let uncompressed = read_u32(&directory, at + 24)? as u64;
        let name_len = read_u16(&directory, at + 28)? as usize;
        let extra_len = read_u16(&directory, at + 30)? as usize;
        let comment_len = read_u16(&directory, at + 32)? as usize;
        let dos_time = read_u16(&directory, at + 12)?;
        let dos_date = read_u16(&directory, at + 14)?;

        let name_at = at + 46;
        let name_bytes = directory.get(name_at..name_at + name_len)?;
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        let is_dir = name.ends_with('/');

        seen += 1;
        if rows.len() >= MAX_ROWS {
            truncated = true;
        } else {
            let display = name.trim_end_matches('/').to_string();
            let leaf = display.rsplit('/').next().unwrap_or(&display).to_string();
            let icon = if is_dir {
                "folder".to_string()
            } else {
                filetype::kind_for_name(&leaf).generic_icon().to_string()
            };
            rows.push(Row {
                name: display,
                size: uncompressed,
                mtime: dos_timestamp(dos_date, dos_time),
                icon,
                is_dir,
            });
        }
        at = name_at + name_len + extra_len + comment_len;
    }

    (!rows.is_empty() || entry_count == 0).then_some((rows, truncated, entry_count))
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// MS-DOS date/time as seconds since the epoch. Approximate by design — it
/// carries no timezone, and a listing shows a date, not a timestamp.
fn dos_timestamp(date: u16, time: u16) -> i64 {
    let year = 1980 + ((date >> 9) & 0x7F) as i64;
    let month = ((date >> 5) & 0x0F) as i64;
    let day = (date & 0x1F) as i64;
    let hour = ((time >> 11) & 0x1F) as i64;
    let minute = ((time >> 5) & 0x3F) as i64;
    let second = ((time & 0x1F) as i64) * 2;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return 0;
    }
    days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
}

/// Days since 1970-01-01 for a proleptic Gregorian date. Howard Hinnant's
/// `days_from_civil`, which is exact and needs no calendar library.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

// ---------------------------------------------------------------------------
// Tar
// ---------------------------------------------------------------------------

/// Uncompressed tar only. A `.tar.gz` reaches here as its compressed type and
/// gets a file card — listing it would need an inflate implementation, which is
/// a dependency v1 declined.
pub fn tar(file: &mut File, metadata: &Metadata, request: &Request) -> PreviewPayload {
    // Headers are every 512 bytes and entries are padded to that, so walking
    // the whole archive means reading it. Cap it: the point is a look inside.
    let bytes = match read_capped(file, 64 * 1024 * 1024) {
        Ok(bytes) => bytes,
        Err(_) => return super::media::generic(metadata, request, "application/x-tar"),
    };

    let mut rows = Vec::new();
    let mut truncated = bytes.len() as u64 == 64 * 1024 * 1024;
    let mut at = 0usize;
    let mut total = 0usize;

    while at + 512 <= bytes.len() {
        let header = &bytes[at..at + 512];
        // Two zero blocks end the archive; one all-zero header is enough to
        // stop walking.
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let Some(name) = tar_string(&header[0..100]) else {
            break;
        };
        if name.is_empty() {
            break;
        }
        let size = tar_octal(&header[124..136]).unwrap_or(0);
        let mtime = tar_octal(&header[136..148]).unwrap_or(0) as i64;
        let type_flag = header[156];
        // Directories are '5'; '0' and NUL are regular files. Everything else
        // (links, devices, and the long-name extensions) is listed as-is.
        let is_dir = type_flag == b'5' || name.ends_with('/');

        total += 1;
        if rows.len() >= MAX_ROWS {
            truncated = true;
        } else {
            let display = name.trim_end_matches('/').to_string();
            let leaf = display.rsplit('/').next().unwrap_or(&display).to_string();
            let icon = if is_dir {
                "folder".to_string()
            } else {
                filetype::kind_for_name(&leaf).generic_icon().to_string()
            };
            rows.push(Row {
                name: display,
                size,
                mtime,
                icon,
                is_dir,
            });
        }

        // Content is padded up to the next 512-byte boundary.
        let advance = 512 + (size as usize).div_ceil(512) * 512;
        at = match at.checked_add(advance) {
            Some(next) if next > at => next,
            _ => break,
        };
    }

    if rows.is_empty() {
        return super::media::generic(metadata, request, "application/x-tar");
    }

    PreviewPayload::Rows {
        rows,
        truncated,
        summary: otto_kit::t_owned!(
            "quickview-archive-summary",
            items = otto_kit::t_owned!("quickview-item-count", count = total as f64),
            size = human_size(metadata.len())
        ),
    }
}

/// A NUL-terminated field from a tar header.
fn tar_string(field: &[u8]) -> Option<String> {
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    Some(String::from_utf8_lossy(&field[..end]).trim().to_string())
}

/// Tar stores numbers as NUL/space-terminated octal text.
fn tar_octal(field: &[u8]) -> Option<u64> {
    let text = tar_string(field)?;
    let digits = text.trim();
    if digits.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(digits, 8).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dos_epoch_is_the_dos_epoch() {
        // 1980-01-01 00:00:00, the earliest a zip can express.
        assert_eq!(dos_timestamp(0b0000_0000_0010_0001, 0), 315_532_800);
    }

    #[test]
    fn an_impossible_dos_date_is_no_date_rather_than_a_wrong_one() {
        assert_eq!(dos_timestamp(0, 0), 0);
    }

    #[test]
    fn civil_days_match_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
    }

    #[test]
    fn tar_octal_reads_padded_fields() {
        assert_eq!(tar_octal(b"00000001750\0"), Some(1000));
        assert_eq!(tar_octal(b"           \0"), Some(0));
        assert_eq!(tar_octal(b"not octal!!\0"), None);
    }

    #[test]
    fn tar_strings_stop_at_the_terminator() {
        assert_eq!(tar_string(b"a.txt\0\0\0\0\0").as_deref(), Some("a.txt"));
    }
}
