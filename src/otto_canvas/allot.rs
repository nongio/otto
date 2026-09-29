//! Sharing the column's height among the canvas items (`max_height`,
//! version 5).
//!
//! Every item that says how tall its content is gets a share of the column:
//! an item whose content fits in an even share keeps its content's height,
//! and the items that want more split what is left equally, again and again
//! until every share is settled (water-filling). Items that do not take
//! part, clients bound below version 5, keep whatever height their buffer
//! has, and the others share what they leave.
//!
//! An item is never squeezed below [`MIN_SHARE`] (or its content, when that
//! is smaller) by the items that do not take part: it keeps that much even
//! if the column then overflows, since those items would be cut at the
//! bottom anyway. Only a column too short to give every item its minimum on
//! its own shares its whole height equally.

// Rust guideline compliant 2026-02-21

/// The least height, in logical points, an item that wants more is given:
/// room for a heading and a couple of rows, so it stays usable next to a
/// tall neighbour.
pub const MIN_SHARE: u32 = 120;

/// What one item asks of the column, in logical points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Demand {
    /// An item that does not take part: it is this tall, whatever it is
    /// given.
    Fixed(u32),
    /// An item whose content is this tall.
    Content(u32),
}

impl Demand {
    fn height(self) -> u32 {
        match self {
            Self::Fixed(h) | Self::Content(h) => h,
        }
    }
}

/// The most each item may be tall, in logical points, in stacking order.
///
/// `available` is the column's height and `gap` the space between two items
/// that show. A [`Demand::Fixed`] item's entry is its own height; a
/// [`Demand::Content`] item's is its share, never more than its content.
///
/// # Examples
///
/// ```
/// use otto::otto_canvas::allot::{allot, Demand};
///
/// // A short item keeps its height; a tall one gets the rest.
/// let shares = allot(1000, 10, &[Demand::Content(200), Demand::Content(2000)]);
/// assert_eq!(shares, vec![200, 790]);
/// ```
pub fn allot(available: u32, gap: u32, demands: &[Demand]) -> Vec<u32> {
    // An item with nothing to show takes no gap under it.
    let showing = demands.iter().filter(|d| d.height() > 0).count();
    let gaps = gap.saturating_mul(u32::try_from(showing.saturating_sub(1)).unwrap_or(u32::MAX));
    let column = available.saturating_sub(gaps);
    let fixed: u32 = demands
        .iter()
        .filter_map(|d| match d {
            Demand::Fixed(h) => Some(*h),
            Demand::Content(_) => None,
        })
        .fold(0, u32::saturating_add);
    let pool = column.saturating_sub(fixed);

    let contents: Vec<(usize, u32)> = demands
        .iter()
        .enumerate()
        .filter_map(|(index, d)| match d {
            Demand::Content(h) => Some((index, *h)),
            Demand::Fixed(_) => None,
        })
        .collect();
    let floors: u32 = contents
        .iter()
        .map(|(_, h)| (*h).min(MIN_SHARE))
        .fold(0, u32::saturating_add);

    let shares: Vec<u32> = if floors <= pool {
        water_fill(pool, &contents)
    } else if floors <= column {
        // The fixed items crowd the others out; those keep their minimum.
        contents.iter().map(|(_, h)| (*h).min(MIN_SHARE)).collect()
    } else {
        // Too short even for the minimums: the column is shared equally.
        water_fill(column, &contents)
    };

    let mut result: Vec<u32> = demands.iter().map(|d| d.height()).collect();
    for ((index, content), share) in contents.iter().zip(shares) {
        // Something to draw, however little room there is.
        result[*index] = if *content > 0 { share.max(1) } else { 0 };
    }
    result
}

/// Share `pool` among `contents` (index, content height), in their order:
/// contents under the level keep their height, the rest get the level.
/// Points left over from the division go to the first items at the level.
fn water_fill(pool: u32, contents: &[(usize, u32)]) -> Vec<u32> {
    let total: u64 = contents.iter().map(|(_, h)| u64::from(*h)).sum();
    if total <= u64::from(pool) {
        return contents.iter().map(|(_, h)| *h).collect();
    }
    let mut ascending: Vec<u32> = contents.iter().map(|(_, h)| *h).collect();
    ascending.sort_unstable();
    let mut remaining = pool;
    let mut left = ascending.len() as u32;
    let mut level = 0;
    for content in ascending {
        let even = remaining / left;
        if content > even {
            level = even;
            break;
        }
        remaining -= content;
        left -= 1;
    }
    // `left` items sit at the level; what the division left over is spread
    // one point at a time.
    let mut spare = remaining - level * left;
    contents
        .iter()
        .map(|(_, h)| {
            if *h <= level {
                *h
            } else {
                let extra = u32::from(spare > 0);
                spare -= extra;
                level + extra
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{allot, Demand, MIN_SHARE};

    fn sum(shares: &[u32]) -> u32 {
        shares.iter().sum()
    }

    #[test]
    fn everything_fits_and_keeps_its_content() {
        let shares = allot(1000, 10, &[Demand::Content(300), Demand::Content(400)]);
        assert_eq!(shares, vec![300, 400]);
    }

    #[test]
    fn a_small_item_keeps_its_height_next_to_a_tall_one() {
        let shares = allot(1000, 10, &[Demand::Content(2000), Demand::Content(200)]);
        assert_eq!(shares, vec![790, 200]);
    }

    #[test]
    fn two_tall_items_split_the_column_evenly() {
        let shares = allot(1001, 10, &[Demand::Content(5000), Demand::Content(900)]);
        // 991 points to share: the first item takes the odd one.
        assert_eq!(shares, vec![496, 495]);
        assert_eq!(sum(&shares) + 10, 1001);
    }

    #[test]
    fn shares_settle_over_several_rounds() {
        // The even share is 242; the 100 fits, which raises it to 290 for
        // the other three, and the 280 fits that, leaving 295 each for the
        // two tallest.
        let shares = allot(
            1000,
            10,
            &[
                Demand::Content(1000),
                Demand::Content(100),
                Demand::Content(280),
                Demand::Content(5000),
            ],
        );
        assert_eq!(shares, vec![295, 100, 280, 295]);
        assert_eq!(sum(&shares) + 30, 1000);
    }

    #[test]
    fn fixed_items_keep_their_height_and_the_rest_share_what_they_leave() {
        let shares = allot(1000, 10, &[Demand::Fixed(300), Demand::Content(2000)]);
        assert_eq!(shares, vec![300, 690]);
    }

    #[test]
    fn an_item_without_a_buffer_takes_no_gap() {
        let shares = allot(1000, 10, &[Demand::Fixed(0), Demand::Content(2000)]);
        assert_eq!(shares, vec![0, 1000]);
    }

    #[test]
    fn fixed_items_cannot_squeeze_an_item_below_its_minimum() {
        let shares = allot(
            1000,
            10,
            &[
                Demand::Fixed(950),
                Demand::Content(2000),
                Demand::Content(60),
            ],
        );
        assert_eq!(shares, vec![950, MIN_SHARE, 60]);
    }

    #[test]
    fn a_tiny_column_is_shared_equally() {
        let shares = allot(200, 10, &[Demand::Content(500), Demand::Content(500)]);
        assert_eq!(shares, vec![95, 95]);
    }

    #[test]
    fn a_tiny_column_still_gives_small_items_their_content() {
        let shares = allot(
            200,
            0,
            &[
                Demand::Content(30),
                Demand::Content(500),
                Demand::Content(500),
            ],
        );
        assert_eq!(shares, vec![30, 85, 85]);
    }

    #[test]
    fn no_room_at_all_still_leaves_something_to_draw() {
        let shares = allot(0, 10, &[Demand::Content(500), Demand::Content(0)]);
        assert_eq!(shares, vec![1, 0]);
    }

    #[test]
    fn shares_never_exceed_the_column_when_the_minimums_fit() {
        for available in [400_u32, 480, 777, 1000, 1400] {
            for contents in [[50_u32, 900, 900], [700, 700, 700], [120, 1, 3000]] {
                let demands: Vec<_> = contents.iter().map(|h| Demand::Content(*h)).collect();
                let shares = allot(available, 8, &demands);
                assert!(
                    sum(&shares) + 16 <= available,
                    "{contents:?} in {available}: {shares:?}"
                );
                for (share, content) in shares.iter().zip(contents) {
                    assert!(*share <= content);
                    assert!(*share >= content.min(MIN_SHARE));
                }
            }
        }
    }
}
