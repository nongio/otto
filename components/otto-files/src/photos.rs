//! The Photos view's model: which entries are pictures, the days they are
//! grouped under, and each picture's proportions.
//!
//! The view itself — the justified rows, the tiles — is [`crate::view`]'s.
//! What is here is what that layout is computed *from*: a heading per day,
//! the trailing section for everything that is not a picture, and the
//! intrinsic size of every picture in the listing, probed off the UI thread
//! from the first few kilobytes of each file.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use otto_kit::filetype::Kind;

use crate::imagesize::{self, Header};
use crate::model::Entry;
use crate::recent;
use crate::view::PhotosSection;
use otto_search::dates::civil_from_days;

/// Whether an entry is a picture: what can be turned, has a palette, and is
/// counted as a photo.
pub fn is_photo(entry: &Entry) -> bool {
    !entry.is_dir && entry.kind == Kind::Image
}

/// Whether an entry is a video.
pub fn is_video(entry: &Entry) -> bool {
    !entry.is_dir && entry.kind == Kind::Video
}

/// Whether an entry is laid out on the wall at its own proportions, its
/// thumbnail filling the tile — a picture, or a video's poster frame — rather
/// than as a square tile in the trailing "Other Files" section.
pub fn is_media(entry: &Entry) -> bool {
    is_photo(entry) || is_video(entry)
}

const DAY: i64 = 86_400;

/// The local calendar day `modified` falls on, as days since the epoch.
///
/// Local, not UTC: a photograph taken at eleven at night belongs to that
/// evening, not to the next day in Greenwich.
pub fn local_day(modified: Option<SystemTime>) -> Option<i64> {
    let secs = recent::epoch_secs(modified?)?;
    Some((secs + otto_search::dates::local_offset(secs)).div_euclid(DAY))
}

/// Today, as [`local_day`] counts days.
pub fn today() -> i64 {
    local_day(Some(SystemTime::now())).unwrap_or(0)
}

/// The full month names, January first.
const MONTHS: [&str; 12] = [
    "files-month-long-jan",
    "files-month-long-feb",
    "files-month-long-mar",
    "files-month-long-apr",
    "files-month-long-may",
    "files-month-long-jun",
    "files-month-long-jul",
    "files-month-long-aug",
    "files-month-long-sep",
    "files-month-long-oct",
    "files-month-long-nov",
    "files-month-long-dec",
];

/// A day's heading: "Friday, 25 September", with the year added when it is
/// not the current one.
pub fn day_heading(day: i64, today: i64) -> String {
    const WEEKDAYS: [&str; 7] = [
        "files-weekday-sun",
        "files-weekday-mon",
        "files-weekday-tue",
        "files-weekday-wed",
        "files-weekday-thu",
        "files-weekday-fri",
        "files-weekday-sat",
    ];
    let (year, month, date) = civil_from_days(day);
    let (this_year, ..) = civil_from_days(today);
    // The epoch was a Thursday, which is index 4 counting from Sunday.
    let weekday = otto_kit::t!(WEEKDAYS[(day + 4).rem_euclid(7) as usize]);
    let month = otto_kit::t!(MONTHS[(month as usize).clamp(1, 12) - 1]);
    if year == this_year {
        otto_kit::t_owned!(
            "files-photos-day",
            weekday = weekday,
            day = date.to_string(),
            month = month
        )
    } else {
        otto_kit::t_owned!(
            "files-photos-day-year",
            weekday = weekday,
            day = date.to_string(),
            month = month,
            year = year.to_string()
        )
    }
}

/// How the Photos view breaks its pictures up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Grouping {
    /// A heading per calendar day.
    #[default]
    Day,
    /// A heading per calendar month.
    Month,
    /// One run of pictures with no headings between them.
    None,
}

impl Grouping {
    pub const ALL: [Grouping; 3] = [Grouping::Day, Grouping::Month, Grouping::None];

    /// The stable name it is remembered by.
    pub fn id(self) -> &'static str {
        match self {
            Grouping::Day => "day",
            Grouping::Month => "month",
            Grouping::None => "none",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.id() == id)
    }

    /// The header control's label for it.
    pub fn label(self) -> &'static str {
        match self {
            Grouping::Day => otto_kit::t!("files-photos-group-day"),
            Grouping::Month => otto_kit::t!("files-photos-group-month"),
            Grouping::None => otto_kit::t!("files-photos-group-none"),
        }
    }
}

/// Where an entry goes in the Photos view: folders first, then pictures, then
/// everything else. The order of the variants is the order on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SectionKind {
    Folders,
    Photos,
    Other,
}

/// Which of the Photos view's kinds of section `entry` belongs in.
pub fn kind_of(entry: &Entry) -> SectionKind {
    if entry.is_dir {
        SectionKind::Folders
    } else if is_media(entry) {
        SectionKind::Photos
    } else {
        SectionKind::Other
    }
}

/// The run a picture falls in under `grouping`, given the day it belongs to
/// (see [`Dims::day`]): that day, its month counted from the epoch, or one
/// run for all. `None` for a picture whose date could not be read.
pub fn group_key(day: Option<i64>, grouping: Grouping) -> Option<i64> {
    match grouping {
        Grouping::None => Some(0),
        Grouping::Day => day,
        Grouping::Month => {
            let (year, month, _) = civil_from_days(day?);
            Some(year * 12 + month as i64 - 1)
        }
    }
}

/// A month's heading: "September 2026", always with its year — a month is
/// broad enough that the year is part of saying which.
pub fn month_heading(key: i64) -> String {
    let month = otto_kit::t!(MONTHS[key.rem_euclid(12) as usize]);
    otto_kit::t_owned!(
        "files-photos-month",
        month = month,
        // The month's number as well, for languages that name a month
        // differently standing alone than inside a date.
        number = key.rem_euclid(12) + 1,
        year = key.div_euclid(12).to_string()
    )
}

/// The heading over a run of pictures.
fn group_heading(key: Option<i64>, grouping: Grouping, today: i64) -> String {
    match (key, grouping) {
        (None, _) => otto_kit::t_owned!("files-photos-undated"),
        (Some(_), Grouping::None) => String::new(),
        (Some(day), Grouping::Day) => day_heading(day, today),
        (Some(month), Grouping::Month) => month_heading(month),
    }
}

/// The Photos view's sections over `entries`, which are in display order:
/// the folders, one section per run of pictures from the same day or month,
/// then everything else.
///
/// Runs rather than buckets. The listing is sorted so each group's pictures
/// are together (see `Browser::ensure_sorted`), so a run is a group — and if
/// one ever were not, a second heading for it is a truer picture of the order
/// than folding tiles under a heading they do not sit beneath.
pub fn sections(
    entries: &[&Entry],
    today: i64,
    grouping: Grouping,
    dims: &Dims,
) -> Vec<PhotosSection> {
    let mut out: Vec<PhotosSection> = Vec::new();
    let mut run: Option<(SectionKind, Option<i64>)> = None;
    for (index, entry) in entries.iter().enumerate() {
        let kind = kind_of(entry);
        let key = match kind {
            SectionKind::Photos => group_key(dims.day(entry), grouping),
            SectionKind::Folders | SectionKind::Other => None,
        };
        match out.last_mut() {
            Some(last) if run == Some((kind, key)) && last.first + last.count == index => {
                last.count += 1;
            }
            _ => {
                run = Some((kind, key));
                let title = match kind {
                    SectionKind::Folders => otto_kit::t_owned!("files-photos-folders-title"),
                    SectionKind::Photos => group_heading(key, grouping, today),
                    SectionKind::Other => otto_kit::t_owned!("files-photos-other"),
                };
                out.push(PhotosSection {
                    title,
                    kind,
                    first: index,
                    count: 1,
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Proportions
// ---------------------------------------------------------------------------

/// The aspect a picture is laid out at before its size is known. Most
/// cameras shoot 3:2 or 4:3, and a guess close to the answer moves the least
/// when the answer lands.
pub const PLACEHOLDER_ASPECT: f32 = 1.5;

/// The aspect a video is laid out at until its header is read, and when it
/// cannot be: 16:9, what nearly everything records at.
pub const VIDEO_ASPECT: f32 = 16.0 / 9.0;

/// The widest and tallest a tile may be, as width over height. A panorama at
/// its true proportions would be a strip too thin to see, and a tall
/// screenshot would take a whole row to itself; past these the thumbnail is
/// cropped to fit instead.
const MIN_ASPECT: f32 = 0.4;
const MAX_ASPECT: f32 = 3.0;

/// How much of a file each probe reads, tried in turn while the header is
/// still further in. Nearly every format declares its size in the first few
/// hundred bytes; the larger reads are for JPEGs that carry a big EXIF
/// preview or XMP block ahead of the frame header.
const PROBE_READS: [u64; 3] = [64 << 10, 1 << 20, 16 << 20];

/// How many files one background probe takes. The layout is recomputed as
/// each batch lands, so a large folder settles in steps rather than all at
/// once at the end.
const BATCH: usize = 128;

/// The most sizes remembered before the table is dropped and refilled from
/// whatever is being looked at.
const CAPACITY: usize = 50_000;

/// A picture's size as it is shown, width then height, with a JPEG's EXIF
/// orientation applied so a portrait shot on a phone held upright is tall.
///
/// Reads the header only, parsed by [`crate::imagesize`] rather than by a
/// codec: these bytes are untrusted, and this runs in the file manager's own
/// process rather than in Peek's sandbox. `None` for a format that reader
/// does not know, which the layout shows square.
///
/// **Blocks** on the file. Belongs on a background thread.
///
/// The day the picture was taken comes back with it, from the EXIF in the
/// same bytes: every format that carries one — JPEG, TIFF and the raws, HEIC
/// — writes it at the head of the file, before the header ends.
pub fn probe(path: &Path) -> (Option<(u32, u32)>, Option<i64>) {
    let Ok(mut file) = std::fs::File::open(path) else {
        return (None, None);
    };
    let mut bytes = Vec::new();
    let mut size = None;
    for limit in PROBE_READS {
        let have = bytes.len() as u64;
        if (&mut file)
            .take(limit - have)
            .read_to_end(&mut bytes)
            .is_err()
        {
            break;
        }
        match imagesize::parse(&bytes) {
            Header::Size(w, h) => {
                size = Some((w, h));
                break;
            }
            Header::Unknown => break,
            // Short of the limit means the whole file is in hand, and there
            // is no more to read.
            Header::NeedMore if (bytes.len() as u64) < limit => break,
            Header::NeedMore => {}
        }
    }
    let taken = crate::camera::parse(&bytes).and_then(|shot| shot.taken);
    (size, taken)
}

/// [`probe`] over bytes already read.
pub fn probe_bytes(bytes: &[u8]) -> Option<(u32, u32)> {
    match imagesize::parse(bytes) {
        Header::Size(w, h) => Some((w, h)),
        Header::NeedMore | Header::Unknown => None,
    }
}

/// What is known about one file's size, and the day it was taken.
struct Known {
    /// The modification time it was probed against.
    modified: Option<SystemTime>,
    /// `None` when the probe could not read a size.
    size: Option<(u32, u32)>,
    /// The day the camera says it was taken, from the same header read.
    taken: Option<i64>,
}

/// One file for the background probe.
pub type ProbeJob = (PathBuf, Option<SystemTime>);

/// One file's answer from the background probe.
pub type ProbeResult = (PathBuf, Option<SystemTime>, Option<(u32, u32)>, Option<i64>);

/// The sizes of the pictures this window has seen.
///
/// Holds no thread: [`Dims::wanted`] hands out a batch and the host probes it
/// and reports through [`Dims::finish`], the same split the thumbnail store
/// uses. One batch is out at a time.
#[derive(Default)]
pub struct Dims {
    known: HashMap<PathBuf, Known>,
    in_flight: bool,
    /// Bumped whenever a batch lands, so the layout that reads these sizes
    /// knows to recompute.
    epoch: u64,
}

impl Dims {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Whether a batch is being probed — the host's cue to keep the frame
    /// loop alive so the relayout paints as it lands.
    pub fn is_busy(&self) -> bool {
        self.in_flight
    }

    /// The width over height to lay `entry` out at, clamped to what a row can
    /// hold. [`PLACEHOLDER_ASPECT`] until it is known; square for a picture
    /// whose size could not be read, and for anything that is not a picture.
    pub fn aspect(&self, entry: &Entry) -> f32 {
        if !is_media(entry) {
            return 1.0;
        }
        // A video whose header could not be read is still a video, and
        // nearly every video is widescreen.
        let unknown = if is_video(entry) { VIDEO_ASPECT } else { 1.0 };
        let placeholder = if is_video(entry) {
            VIDEO_ASPECT
        } else {
            PLACEHOLDER_ASPECT
        };
        let aspect = match self.known.get(&entry.path) {
            Some(known) if known.modified == entry.modified => match known.size {
                Some((w, h)) => w as f32 / h as f32,
                None => unknown,
            },
            _ => placeholder,
        };
        aspect.clamp(MIN_ASPECT, MAX_ASPECT)
    }

    /// The next batch of pictures in `entries` whose size is not yet known,
    /// or nothing while a batch is already out.
    ///
    /// A picture whose modification time has not been read yet is left for
    /// later: its size would be recorded against no time at all, and probed
    /// again the moment the time arrived.
    pub fn wanted<'a>(&mut self, entries: impl IntoIterator<Item = &'a Entry>) -> Vec<ProbeJob> {
        if self.in_flight {
            return Vec::new();
        }
        let jobs: Vec<ProbeJob> = entries
            .into_iter()
            .filter(|entry| is_media(entry) && entry.modified.is_some())
            .filter(|entry| {
                self.known
                    .get(&entry.path)
                    .is_none_or(|known| known.modified != entry.modified)
            })
            .take(BATCH)
            .map(|entry| (entry.path.clone(), entry.modified))
            .collect();
        self.in_flight = !jobs.is_empty();
        jobs
    }

    /// `entry`'s size as it is shown, if it has been read.
    pub fn size(&self, entry: &Entry) -> Option<(u32, u32)> {
        self.known
            .get(&entry.path)
            .filter(|known| known.modified == entry.modified)
            .and_then(|known| known.size)
    }

    /// The day `entry` belongs to in the Photos view: the day it was taken,
    /// when the camera wrote it down and it has been read, else the local day
    /// it was last modified.
    ///
    /// Taken first because a modification time is a fact about the file, not
    /// the photograph: copying a card, unpacking an archive or editing a tag
    /// sets it, and a folder of holiday pictures unzipped last week — or
    /// stamped 1980 by the zip — would otherwise be filed under that.
    pub fn day(&self, entry: &Entry) -> Option<i64> {
        self.known
            .get(&entry.path)
            .filter(|known| known.modified == entry.modified)
            .and_then(|known| known.taken)
            .or_else(|| local_day(entry.modified))
    }

    /// Record a batch's answers.
    pub fn finish(&mut self, results: Vec<ProbeResult>) {
        self.in_flight = false;
        if self.known.len() + results.len() > CAPACITY {
            self.known.clear();
        }
        for (path, modified, size, taken) in results {
            self.known.insert(
                path,
                Known {
                    modified,
                    size,
                    taken,
                },
            );
        }
        self.epoch = self.epoch.wrapping_add(1);
    }
}

/// Probe a batch. **Blocks**; run it off the UI thread.
pub fn probe_all(jobs: Vec<ProbeJob>) -> Vec<ProbeResult> {
    jobs.into_iter()
        .map(|(path, modified)| {
            let (size, taken) = if is_video_path(&path) {
                probe_video(&path)
            } else {
                probe(&path)
            };
            (path, modified, size, taken)
        })
        .collect()
}

/// Whether a probe job is a video. Named rather than sniffed: the job
/// carries a path, and its entry was laid out as a video by the same name.
fn is_video_path(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        otto_kit::filetype::kind_for_name(&name.to_string_lossy()) == Kind::Video
    })
}

/// [`probe`] for a video: its size as shown, and the day it was recorded,
/// from the container's header.
pub fn probe_video(path: &Path) -> (Option<(u32, u32)>, Option<i64>) {
    let Some(video) = crate::videosize::read(path) else {
        return (None, None);
    };
    let day = video
        .recorded
        .and_then(|secs| u64::try_from(secs).ok())
        .map(|secs| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs));
    (video.size, local_day(day))
}

// ---------------------------------------------------------------------------
// Folder cards
// ---------------------------------------------------------------------------

/// How many pictures a folder card shows: one large, two small.
pub const PREVIEW_IMAGES: usize = 3;

/// The most entries read from one folder while looking for its newest
/// pictures. A card is a glimpse, and a folder of a hundred thousand files
/// is not worth listing in full for one.
const PREVIEW_SCAN: usize = 4_000;

/// How many folders one background pass looks into.
const FOLDER_BATCH: usize = 16;

/// A picture on a folder card: its path and modification time, which is what
/// the thumbnail store keys on.
pub type PreviewImage = (PathBuf, Option<SystemTime>);

/// One folder for the background pass, by its path and modification time.
pub type FolderJob = (PathBuf, Option<SystemTime>);

/// What the pass found in one folder: its newest pictures, and how many
/// entries it holds (hidden ones aside, and no more than the pass reads).
pub type FolderResult = (PathBuf, Option<SystemTime>, Vec<PreviewImage>, usize);

/// The newest pictures directly inside `dir`, newest first, at most
/// [`PREVIEW_IMAGES`] of them. Only the folder itself is read, never what is
/// below it, and only its first [`PREVIEW_SCAN`] entries.
///
/// **Blocks** on the directory. Belongs on a background thread.
pub fn preview_of(dir: &Path) -> (Vec<PreviewImage>, usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (Vec::new(), 0);
    };
    let shown: Vec<std::fs::DirEntry> = entries
        .take(PREVIEW_SCAN)
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .collect();
    let count = shown.len();
    let mut found: Vec<(SystemTime, PathBuf)> = shown
        .into_iter()
        .filter(|entry| {
            otto_kit::filetype::kind_for_name(&entry.file_name().to_string_lossy()) == Kind::Image
        })
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            meta.is_file().then(|| {
                (
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    entry.path(),
                )
            })
        })
        .collect();
    found.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    let images = found
        .into_iter()
        .take(PREVIEW_IMAGES)
        .map(|(modified, path)| (path, Some(modified)))
        .collect();
    (images, count)
}

/// Look into a batch of folders. **Blocks**; run it off the UI thread.
pub fn preview_all(jobs: Vec<FolderJob>) -> Vec<FolderResult> {
    jobs.into_iter()
        .map(|(path, modified)| {
            let (images, count) = preview_of(&path);
            (path, modified, images, count)
        })
        .collect()
}

/// The pictures each folder card shows, as the background pass finds them.
///
/// Keyed by the folder's own modification time, which moves whenever a file
/// is added to or removed from it — exactly when its newest pictures may have
/// changed. Same split as [`Dims`]: one batch out at a time, the host runs it.
#[derive(Default)]
pub struct FolderPreviews {
    known: HashMap<PathBuf, (Option<SystemTime>, Vec<PreviewImage>, usize)>,
    in_flight: bool,
}

impl FolderPreviews {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_busy(&self) -> bool {
        self.in_flight
    }

    /// The pictures to show on `folder`'s card. `None` until it has been
    /// looked into; empty for a folder with no pictures in it.
    pub fn images(&self, folder: &Entry) -> Option<&[PreviewImage]> {
        match self.known.get(&folder.path) {
            Some((modified, images, _)) if *modified == folder.modified => Some(images),
            _ => None,
        }
    }

    /// How many entries `folder` holds, once it has been looked into.
    pub fn count(&self, folder: &Entry) -> Option<usize> {
        match self.known.get(&folder.path) {
            Some((modified, _, count)) if *modified == folder.modified => Some(*count),
            _ => None,
        }
    }

    /// The next folders in `entries` to look into, or nothing while a batch
    /// is out.
    pub fn wanted<'a>(&mut self, entries: impl IntoIterator<Item = &'a Entry>) -> Vec<FolderJob> {
        if self.in_flight {
            return Vec::new();
        }
        let jobs: Vec<FolderJob> = entries
            .into_iter()
            .filter(|entry| entry.is_dir && self.images(entry).is_none())
            .take(FOLDER_BATCH)
            .map(|entry| (entry.path.clone(), entry.modified))
            .collect();
        self.in_flight = !jobs.is_empty();
        jobs
    }

    /// Record a batch's findings.
    pub fn finish(&mut self, results: Vec<FolderResult>) {
        self.in_flight = false;
        if self.known.len() + results.len() > CAPACITY {
            self.known.clear();
        }
        for (path, modified, images, count) in results {
            self.known.insert(path, (modified, images, count));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn entry(name: &str, kind: Kind, modified: Option<SystemTime>) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{name}")),
            is_dir: kind == Kind::Folder,
            is_symlink: false,
            hidden: false,
            kind,
            size: Some(1),
            modified,
            origin: None,
        }
    }

    /// Noon, local time, on day `day` since the epoch — well clear of either
    /// midnight whatever the time zone.
    fn noon(day: i64) -> SystemTime {
        let guess = day * DAY + DAY / 2;
        let secs = guess - otto_search::dates::local_offset(guess);
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs as u64)
    }

    #[test]
    fn the_heading_names_the_weekday_and_only_a_past_years_year() {
        // 25 September 2026 was a Friday.
        let day = 20_721;
        assert_eq!(civil_from_days(day), (2026, 9, 25));
        assert_eq!(day_heading(day, day + 2), "Friday, 25 September");
        assert_eq!(day_heading(day, day + 400), "Friday, 25 September 2026");
    }

    fn shape(sections: &[PhotosSection]) -> Vec<(SectionKind, usize, usize)> {
        sections
            .iter()
            .map(|s| (s.kind, s.first, s.count))
            .collect()
    }

    #[test]
    fn folders_come_first_then_pictures_by_day_then_everything_else() {
        let owned = [
            entry("Holiday", Kind::Folder, None),
            entry("a.jpg", Kind::Image, Some(noon(20_721))),
            entry("b.jpg", Kind::Image, Some(noon(20_721))),
            entry("c.png", Kind::Image, Some(noon(20_720))),
            entry("notes.txt", Kind::Text, Some(noon(20_721))),
        ];
        let refs: Vec<&Entry> = owned.iter().collect();
        let sections = sections(&refs, 20_722, Grouping::Day, &Dims::new());
        assert_eq!(
            shape(&sections),
            vec![
                (SectionKind::Folders, 0, 1),
                (SectionKind::Photos, 1, 2),
                (SectionKind::Photos, 3, 1),
                (SectionKind::Other, 4, 1),
            ]
        );
        assert_eq!(sections[0].title, "Folders");
        assert_eq!(sections[1].title, "Friday, 25 September");
        assert_eq!(sections[2].title, "Thursday, 24 September");
        assert_eq!(sections[3].title, "Other Files");
    }

    #[test]
    fn a_month_or_no_grouping_makes_fewer_runs() {
        let owned = [
            entry("a.jpg", Kind::Image, Some(noon(20_721))),
            entry("b.jpg", Kind::Image, Some(noon(20_700))),
            entry("c.jpg", Kind::Image, Some(noon(20_690))),
        ];
        let refs: Vec<&Entry> = owned.iter().collect();
        // 20,721 and 20,700 are both September 2026; 20,690 is August.
        let months = sections(&refs, 20_722, Grouping::Month, &Dims::new());
        assert_eq!(
            shape(&months),
            vec![(SectionKind::Photos, 0, 2), (SectionKind::Photos, 2, 1)]
        );
        assert_eq!(months[0].title, "September 2026");
        assert_eq!(months[1].title, "August 2026");

        let one = sections(&refs, 20_722, Grouping::None, &Dims::new());
        assert_eq!(shape(&one), vec![(SectionKind::Photos, 0, 3)]);
        assert!(one[0].title.is_empty());
    }

    #[test]
    fn a_folder_card_shows_its_newest_pictures() {
        let dir = std::env::temp_dir().join(format!("otto-photos-card-{}", std::process::id()));
        let inner = dir.join("Trip");
        std::fs::create_dir_all(inner.join("deeper")).unwrap();
        for (i, name) in ["a.jpg", "b.png", "c.jpg", "d.jpg"].iter().enumerate() {
            let path = inner.join(name);
            std::fs::write(&path, b"x").unwrap();
            let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000 + i as u64 * 100);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        std::fs::write(inner.join("notes.txt"), b"x").unwrap();
        std::fs::write(inner.join("deeper/e.jpg"), b"x").unwrap();

        let (images, count) = preview_of(&inner);
        // Four pictures, the notes and the folder below.
        assert_eq!(count, 6);
        let names: Vec<String> = images
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["d.jpg", "c.jpg", "b.png"]);

        let mut folder = entry("Trip", Kind::Folder, Some(noon(20_000)));
        folder.path = inner.clone();
        let mut previews = FolderPreviews::new();
        assert!(previews.images(&folder).is_none());
        let jobs = previews.wanted([&folder]);
        assert_eq!(jobs.len(), 1);
        assert!(previews.wanted([&folder]).is_empty(), "one batch at a time");
        previews.finish(preview_all(jobs));
        assert_eq!(previews.images(&folder).map(<[_]>::len), Some(3));
        assert_eq!(previews.count(&folder), Some(6));
        assert!(previews.wanted([&folder]).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_grouping_survives_its_name() {
        for grouping in Grouping::ALL {
            assert_eq!(Grouping::from_id(grouping.id()), Some(grouping));
        }
        assert_eq!(Grouping::from_id("week"), None);
    }

    #[test]
    fn a_day_boundary_is_local_midnight() {
        let day = local_day(Some(noon(20_000))).unwrap();
        assert_eq!(day, 20_000);
    }

    fn png(width: i32, height: i32) -> Vec<u8> {
        let mut surface = skia_safe::surfaces::raster_n32_premul((width, height)).unwrap();
        surface.canvas().clear(skia_safe::Color::RED);
        surface
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::PNG, None)
            .unwrap()
            .as_bytes()
            .to_vec()
    }

    fn jpeg(width: i32, height: i32) -> Vec<u8> {
        let mut surface = skia_safe::surfaces::raster_n32_premul((width, height)).unwrap();
        surface.canvas().clear(skia_safe::Color::BLUE);
        surface
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::JPEG, 80)
            .unwrap()
            .as_bytes()
            .to_vec()
    }

    /// `jpeg` with an EXIF block saying the picture is rotated a quarter turn.
    fn rotated_jpeg(width: i32, height: i32) -> Vec<u8> {
        let plain = jpeg(width, height);
        let mut exif: Vec<u8> = b"Exif\0\0".to_vec();
        // Big-endian TIFF header, first IFD at offset 8.
        exif.extend_from_slice(b"MM\0\x2a\0\0\0\x08");
        // One entry: Orientation (0x0112), SHORT, count 1, value 6.
        exif.extend_from_slice(&[0, 1]);
        exif.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]);
        exif.extend_from_slice(&[0, 0, 0, 0]);
        let len = (exif.len() + 2) as u16;
        let mut out = plain[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&exif);
        out.extend_from_slice(&plain[2..]);
        out
    }

    #[test]
    fn the_probe_reads_a_size_from_the_header() {
        assert_eq!(probe_bytes(&png(30, 20)), Some((30, 20)));
        assert_eq!(probe_bytes(&jpeg(40, 10)), Some((40, 10)));
        assert_eq!(probe_bytes(b"not a picture"), None);
    }

    #[test]
    fn the_probe_turns_a_rotated_photo_upright() {
        assert_eq!(probe_bytes(&rotated_jpeg(40, 10)), Some((10, 40)));
    }

    #[test]
    fn the_probe_reads_a_file() {
        let dir = std::env::temp_dir().join(format!("otto-photos-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tall.png");
        std::fs::write(&path, png(12, 48)).unwrap();
        assert_eq!(probe(&path), (Some((12, 48)), None));
        let junk = dir.join("junk.png");
        std::fs::write(&junk, b"nope").unwrap();
        assert_eq!(probe(&junk), (None, None));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sizes_are_asked_for_once_and_used_when_they_land() {
        let when = Some(noon(20_000));
        let owned = [
            entry("a.jpg", Kind::Image, when),
            entry("b.txt", Kind::Text, when),
            entry("c.jpg", Kind::Image, None),
        ];
        let mut dims = Dims::new();
        assert_eq!(dims.aspect(&owned[0]), PLACEHOLDER_ASPECT);
        assert_eq!(dims.aspect(&owned[1]), 1.0);

        let jobs = dims.wanted(owned.iter());
        assert_eq!(jobs, vec![(owned[0].path.clone(), when)]);
        assert!(dims.is_busy());
        assert!(dims.wanted(owned.iter()).is_empty(), "one batch at a time");

        let epoch = dims.epoch();
        dims.finish(vec![(owned[0].path.clone(), when, Some((10, 40)), None)]);
        assert_ne!(dims.epoch(), epoch);
        assert_eq!(dims.aspect(&owned[0]), MIN_ASPECT, "clamped");
        assert!(dims.wanted(owned.iter()).is_empty(), "nothing left to ask");
    }
}
