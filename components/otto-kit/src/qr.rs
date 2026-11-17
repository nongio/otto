//! QR codes: text in, a square of dark and light modules out.
//!
//! Enough of ISO/IEC 18004 for what a desktop shows a phone: byte mode, error
//! correction level M (about 15% of the symbol can be lost), versions 1 to 10
//! (21 to 57 modules a side, up to 213 bytes). The mask is chosen by the
//! standard's penalty rules. [`crate::components::qr_code`] draws one.
//!
//! The steps follow the standard's order: the data is put in a bit stream
//! and padded to the version's capacity, split into blocks that each get
//! Reed–Solomon error correction, interleaved, and laid in the symbol's
//! zigzag around the finder, timing and alignment patterns; then each of the
//! eight masks is tried and the one with the lowest penalty kept.

// Rust guideline compliant 2026-02-21

use std::fmt;

/// The largest version encoded.
const MAX_VERSION: usize = 10;
/// Error correction codewords per block at level M, by version (index 0
/// unused).
const ECC_PER_BLOCK: [usize; MAX_VERSION + 1] = [0, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26];
/// Error correction blocks at level M, by version (index 0 unused).
const BLOCKS: [usize; MAX_VERSION + 1] = [0, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5];
/// Level M's two bits in the format information.
const LEVEL_M_BITS: u32 = 0b00;

/// The penalty weights of the standard's four mask rules.
const PENALTY_RUN: i32 = 3;
const PENALTY_BLOCK: i32 = 3;
const PENALTY_FINDER: i32 = 40;
const PENALTY_BALANCE: i32 = 10;

/// An encoded symbol. Its [`Debug`] says only its size: the modules carry
/// whatever was encoded, which may be a password.
#[derive(Clone, PartialEq, Eq)]
pub struct QrCode {
    size: usize,
    /// Row by row, `true` for dark.
    modules: Vec<bool>,
}

impl fmt::Debug for QrCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QrCode({0}×{0})", self.size)
    }
}

impl QrCode {
    /// Encode `data` in the smallest version that holds it.
    ///
    /// `None` when it does not fit version 10 at level M: more than 213
    /// bytes.
    pub fn encode(data: &[u8]) -> Option<Self> {
        encode_masked(data, None)
    }

    /// Modules a side, without the quiet zone a reader needs around it
    /// (four modules of light).
    pub fn size(&self) -> usize {
        self.size
    }

    /// Whether the module at column `x`, row `y` is dark. Outside the symbol
    /// is light.
    pub fn dark(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.modules[y * self.size + x]
    }
}

/// Encode `data` with `mask`, or with the mask of lowest penalty for `None`.
fn encode_masked(data: &[u8], mask: Option<u32>) -> Option<QrCode> {
    let version = (1..=MAX_VERSION).find(|&v| fits(data.len(), v))?;
    let mut symbol = Symbol::new(version);
    symbol.draw_function_patterns();
    symbol.draw_codewords(&codewords(data, version));
    let mask = mask.unwrap_or_else(|| {
        (0..8)
            .min_by_key(|&mask| {
                symbol.apply_mask(mask);
                symbol.draw_format_bits(mask);
                let penalty = symbol.penalty();
                symbol.apply_mask(mask);
                penalty
            })
            .expect("eight masks")
    });
    symbol.apply_mask(mask);
    symbol.draw_format_bits(mask);
    Some(symbol.finish())
}

/// Bits in the character count of byte mode at `version`.
fn count_bits(version: usize) -> usize {
    if version < 10 {
        8
    } else {
        16
    }
}

/// Modules in `version` left for data and error correction, once the
/// function patterns, format and version information are placed.
fn raw_data_modules(version: usize) -> usize {
    let mut result = (16 * version + 128) * version + 64;
    if version >= 2 {
        let align = version / 7 + 2;
        result -= (25 * align - 10) * align - 55;
        if version >= 7 {
            result -= 36;
        }
    }
    result
}

/// Data codewords `version` holds at level M.
fn data_codewords(version: usize) -> usize {
    raw_data_modules(version) / 8 - ECC_PER_BLOCK[version] * BLOCKS[version]
}

/// Whether `len` bytes fit `version` in byte mode.
fn fits(len: usize, version: usize) -> bool {
    len < (1 << count_bits(version))
        && 4 + count_bits(version) + 8 * len <= data_codewords(version) * 8
}

/// The data codewords for `data` at `version`, padded to its capacity, with
/// the error correction added and the blocks interleaved.
fn codewords(data: &[u8], version: usize) -> Vec<u8> {
    let capacity = data_codewords(version) * 8;
    let mut bits = BitStream::default();
    bits.push(0b0100, 4);
    bits.push(data.len() as u32, count_bits(version));
    for byte in data {
        bits.push(u32::from(*byte), 8);
    }
    // The terminator, as much of it as fits, then up to a whole byte.
    let terminator = (capacity - bits.len).min(4);
    bits.push(0, terminator);
    bits.push(0, (8 - bits.len % 8) % 8);
    let mut bytes = bits.bytes;
    for pad in [0xEC, 0x11].into_iter().cycle() {
        if bytes.len() * 8 >= capacity {
            break;
        }
        bytes.push(pad);
    }
    add_error_correction(&bytes, version)
}

/// A growing string of bits, most significant first.
#[derive(Default)]
struct BitStream {
    bytes: Vec<u8>,
    len: usize,
}

impl BitStream {
    /// Append the low `count` bits of `value`.
    fn push(&mut self, value: u32, count: usize) {
        for i in (0..count).rev() {
            if self.len.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (value >> i) & 1 == 1 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.len % 8);
            }
            self.len += 1;
        }
    }
}

/// Split `data` into the version's blocks, add each block's error
/// correction, and interleave them as the symbol stores them: the first
/// codeword of every block, then the second, and so on, data before error
/// correction. Short blocks come first and are one data codeword shorter.
fn add_error_correction(data: &[u8], version: usize) -> Vec<u8> {
    let blocks = BLOCKS[version];
    let ecc_len = ECC_PER_BLOCK[version];
    let raw = raw_data_modules(version) / 8;
    let short_blocks = blocks - raw % blocks;
    let short_len = raw / blocks;
    let divisor = rs_divisor(ecc_len);

    let mut split = Vec::with_capacity(blocks);
    let mut at = 0;
    for i in 0..blocks {
        let len = short_len - ecc_len + usize::from(i >= short_blocks);
        let block = &data[at..at + len];
        at += len;
        split.push((block, rs_remainder(block, &divisor)));
    }

    let mut out = Vec::with_capacity(raw);
    for i in 0..=short_len - ecc_len {
        for (block, _) in &split {
            if let Some(byte) = block.get(i) {
                out.push(*byte);
            }
        }
    }
    for i in 0..ecc_len {
        for (_, ecc) in &split {
            out.push(ecc[i]);
        }
    }
    out
}

/// Multiply in GF(2⁸) modulo the standard's polynomial, x⁸+x⁴+x³+x²+1.
fn gf_mul(x: u8, y: u8) -> u8 {
    let mut z: u32 = 0;
    for i in (0..8).rev() {
        z = (z << 1) ^ ((z >> 7) * 0x11D);
        z ^= u32::from((y >> i) & 1) * u32::from(x);
    }
    z as u8
}

/// The Reed–Solomon generator polynomial of `degree`, its leading 1 left
/// out, highest power first.
fn rs_divisor(degree: usize) -> Vec<u8> {
    let mut result = vec![0u8; degree];
    result[degree - 1] = 1;
    let mut root = 1u8;
    for _ in 0..degree {
        for j in 0..degree {
            result[j] = gf_mul(result[j], root);
            if j + 1 < degree {
                result[j] ^= result[j + 1];
            }
        }
        root = gf_mul(root, 0x02);
    }
    result
}

/// The error correction codewords of `data`: its remainder by `divisor`.
fn rs_remainder(data: &[u8], divisor: &[u8]) -> Vec<u8> {
    let mut result = vec![0u8; divisor.len()];
    for byte in data {
        let factor = byte ^ result.remove(0);
        result.push(0);
        for (r, d) in result.iter_mut().zip(divisor) {
            *r ^= gf_mul(*d, factor);
        }
    }
    result
}

/// The 15 format bits for level M and `mask`, BCH-protected and masked.
fn format_bits(mask: u32) -> u32 {
    let data = (LEVEL_M_BITS << 3) | mask;
    let mut rem = data;
    for _ in 0..10 {
        rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    ((data << 10) | rem) ^ 0x5412
}

/// The 18 version bits for `version` (7 and up), BCH-protected.
fn version_bits(version: usize) -> u32 {
    let version = version as u32;
    let mut rem = version;
    for _ in 0..12 {
        rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
    }
    (version << 12) | rem
}

/// The centres of the alignment patterns along each axis.
fn alignment_positions(version: usize) -> Vec<usize> {
    if version == 1 {
        return Vec::new();
    }
    let count = version / 7 + 2;
    let step = (version * 4 + count * 2 + 1) / (count * 2 - 2) * 2;
    let mut positions = vec![6];
    let mut at = version * 4 + 10;
    for _ in 0..count - 1 {
        positions.insert(1, at);
        at -= step;
    }
    positions
}

/// A symbol being built.
struct Symbol {
    version: usize,
    size: usize,
    dark: Vec<bool>,
    /// Modules of the finder, timing, alignment, format and version
    /// patterns, which data and masks leave alone.
    function: Vec<bool>,
}

impl Symbol {
    fn new(version: usize) -> Self {
        let size = version * 4 + 17;
        Self {
            version,
            size,
            dark: vec![false; size * size],
            function: vec![false; size * size],
        }
    }

    fn finish(self) -> QrCode {
        QrCode {
            size: self.size,
            modules: self.dark,
        }
    }

    fn get(&self, x: usize, y: usize) -> bool {
        self.dark[y * self.size + x]
    }

    fn set_function(&mut self, x: usize, y: usize, dark: bool) {
        let at = y * self.size + x;
        self.dark[at] = dark;
        self.function[at] = true;
    }

    fn draw_function_patterns(&mut self) {
        let size = self.size;
        for i in 0..size {
            self.set_function(6, i, i % 2 == 0);
            self.set_function(i, 6, i % 2 == 0);
        }
        self.draw_finder(3, 3);
        self.draw_finder(size - 4, 3);
        self.draw_finder(3, size - 4);

        let positions = alignment_positions(self.version);
        let last = positions.len().saturating_sub(1);
        for (i, &x) in positions.iter().enumerate() {
            for (j, &y) in positions.iter().enumerate() {
                // The three corners the finders hold.
                let corner = (i == 0 && j == 0) || (i == 0 && j == last) || (i == last && j == 0);
                if !corner {
                    self.draw_alignment(x, y);
                }
            }
        }

        // Reserved now, written once the mask is known.
        self.draw_format_bits(0);
        self.draw_version();
    }

    /// A finder pattern centred on (`cx`, `cy`), with its light separator.
    fn draw_finder(&mut self, cx: usize, cy: usize) {
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                if (0..self.size as i32).contains(&x) && (0..self.size as i32).contains(&y) {
                    let ring = dx.abs().max(dy.abs());
                    self.set_function(x as usize, y as usize, ring != 2 && ring != 4);
                }
            }
        }
    }

    fn draw_alignment(&mut self, cx: usize, cy: usize) {
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                let ring = dx.abs().max(dy.abs());
                self.set_function(
                    (cx as i32 + dx) as usize,
                    (cy as i32 + dy) as usize,
                    ring != 1,
                );
            }
        }
    }

    /// Both copies of the format information, and the dark module.
    fn draw_format_bits(&mut self, mask: u32) {
        let bits = format_bits(mask);
        let bit = |i: u32| (bits >> i) & 1 == 1;
        let size = self.size;
        // Around the top-left finder.
        for i in 0..6 {
            self.set_function(8, i, bit(i as u32));
        }
        self.set_function(8, 7, bit(6));
        self.set_function(8, 8, bit(7));
        self.set_function(7, 8, bit(8));
        for i in 9..15 {
            self.set_function(14 - i, 8, bit(i as u32));
        }
        // Split between the other two finders.
        for i in 0..8 {
            self.set_function(size - 1 - i, 8, bit(i as u32));
        }
        for i in 8..15 {
            self.set_function(8, size - 15 + i, bit(i as u32));
        }
        self.set_function(8, size - 8, true);
    }

    /// Both copies of the version information, from version 7.
    fn draw_version(&mut self) {
        if self.version < 7 {
            return;
        }
        let bits = version_bits(self.version);
        for i in 0..18 {
            let dark = (bits >> i) & 1 == 1;
            let a = self.size - 11 + i % 3;
            let b = i / 3;
            self.set_function(a, b, dark);
            self.set_function(b, a, dark);
        }
    }

    /// Lay the codewords in the zigzag: two columns at a time from the
    /// right, up then down, skipping the vertical timing pattern.
    fn draw_codewords(&mut self, codewords: &[u8]) {
        let total = codewords.len() * 8;
        let mut i = 0;
        let mut right = self.size as i32 - 1;
        while right >= 1 {
            if right == 6 {
                right = 5;
            }
            for vert in 0..self.size {
                for j in 0..2 {
                    let x = (right - j) as usize;
                    let upward = (right + 1) & 2 == 0;
                    let y = if upward { self.size - 1 - vert } else { vert };
                    let at = y * self.size + x;
                    if !self.function[at] && i < total {
                        self.dark[at] = (codewords[i >> 3] >> (7 - (i & 7))) & 1 == 1;
                        i += 1;
                    }
                    // Modules past the codewords (the remainder bits) stay
                    // light, before masking.
                }
            }
            right -= 2;
        }
    }

    /// Flip the data modules `mask` selects. Applying it twice undoes it.
    fn apply_mask(&mut self, mask: u32) {
        for y in 0..self.size {
            for x in 0..self.size {
                let flip = match mask {
                    0 => (x + y) % 2 == 0,
                    1 => y % 2 == 0,
                    2 => x % 3 == 0,
                    3 => (x + y) % 3 == 0,
                    4 => (x / 3 + y / 2) % 2 == 0,
                    5 => x * y % 2 + x * y % 3 == 0,
                    6 => (x * y % 2 + x * y % 3) % 2 == 0,
                    _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
                };
                let at = y * self.size + x;
                if flip && !self.function[at] {
                    self.dark[at] = !self.dark[at];
                }
            }
        }
    }

    /// The standard's penalty for the symbol as it stands: runs of one
    /// colour, 2×2 blocks, finder-like patterns, and dark/light balance.
    fn penalty(&self) -> i32 {
        let size = self.size;
        let mut result = 0;
        for line in 0..size {
            for horizontal in [true, false] {
                let module = |i: usize| {
                    if horizontal {
                        self.get(i, line)
                    } else {
                        self.get(line, i)
                    }
                };
                let mut run_dark = false;
                let mut run = 0usize;
                let mut history = RunHistory::new(size);
                for i in 0..size {
                    if module(i) == run_dark {
                        run += 1;
                        if run == 5 {
                            result += PENALTY_RUN;
                        } else if run > 5 {
                            result += 1;
                        }
                    } else {
                        history.add(run);
                        if !run_dark {
                            result += history.finder_like() * PENALTY_FINDER;
                        }
                        run_dark = module(i);
                        run = 1;
                    }
                }
                result += history.terminate(run_dark, run) * PENALTY_FINDER;
            }
        }

        for y in 0..size - 1 {
            for x in 0..size - 1 {
                let c = self.get(x, y);
                if c == self.get(x + 1, y) && c == self.get(x, y + 1) && c == self.get(x + 1, y + 1)
                {
                    result += PENALTY_BLOCK;
                }
            }
        }

        let dark = self.dark.iter().filter(|d| **d).count() as i32;
        let total = (size * size) as i32;
        // The smallest k with |dark/total − 1/2| ≤ (k + 1)/20.
        let k = ((dark * 20 - total * 10).abs() + total - 1) / total - 1;
        result + k * PENALTY_BALANCE
    }
}

/// The last seven run lengths along a line, newest first, for spotting the
/// 1:1:3:1:1 finder pattern with light on either side.
struct RunHistory {
    size: usize,
    runs: [usize; 7],
}

impl RunHistory {
    fn new(size: usize) -> Self {
        Self { size, runs: [0; 7] }
    }

    fn add(&mut self, mut run: usize) {
        // The first run is light and continues into the quiet zone.
        if self.runs[0] == 0 {
            run += self.size;
        }
        self.runs.copy_within(0..6, 1);
        self.runs[0] = run;
    }

    /// How many finder-like patterns end at the newest run: with four
    /// modules of light before or after.
    fn finder_like(&self) -> i32 {
        let h = &self.runs;
        let n = h[1];
        let core = n > 0 && h[2] == n && h[3] == n * 3 && h[4] == n && h[5] == n;
        i32::from(core && h[0] >= n * 4 && h[6] >= n)
            + i32::from(core && h[6] >= n * 4 && h[0] >= n)
    }

    /// Close the line, counting the quiet zone after it as light.
    fn terminate(&mut self, run_dark: bool, mut run: usize) -> i32 {
        if run_dark {
            self.add(run);
            run = 0;
        }
        run += self.size;
        self.add(run);
        self.finder_like()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reed_solomon_matches_the_worked_example() {
        // "HELLO WORLD" at 1-M, as the standard's tutorials work it.
        let data = [
            32, 91, 11, 120, 209, 114, 220, 77, 67, 64, 236, 17, 236, 17, 236, 17,
        ];
        assert_eq!(
            rs_remainder(&data, &rs_divisor(10)),
            [196, 35, 39, 119, 235, 215, 231, 226, 93, 23]
        );
    }

    #[test]
    fn format_and_version_bits_match_the_standard() {
        let level_m = [
            0b101010000010010,
            0b101000100100101,
            0b101111001111100,
            0b101101101001011,
            0b100010111111001,
            0b100000011001110,
            0b100111110010111,
            0b100101010100000,
        ];
        for (mask, bits) in level_m.into_iter().enumerate() {
            assert_eq!(format_bits(mask as u32), bits, "mask {mask}");
        }
        assert_eq!(version_bits(7), 0b000111110010010100);
        assert_eq!(version_bits(10), 0b001010010011010011);
    }

    #[test]
    fn capacities_at_level_m() {
        let data: Vec<usize> = (1..=MAX_VERSION).map(data_codewords).collect();
        assert_eq!(data, [16, 28, 44, 64, 86, 108, 124, 154, 182, 216]);
        let total: Vec<usize> = (1..=MAX_VERSION).map(|v| raw_data_modules(v) / 8).collect();
        assert_eq!(total, [26, 44, 70, 100, 134, 172, 196, 242, 292, 346]);
        assert_eq!(alignment_positions(2), [6, 18]);
        assert_eq!(alignment_positions(7), [6, 22, 38]);
        assert_eq!(alignment_positions(10), [6, 28, 50]);
    }

    #[test]
    fn the_smallest_version_that_holds_it() {
        assert_eq!(QrCode::encode(b"").unwrap().size(), 21);
        assert_eq!(QrCode::encode(&[b'a'; 14]).unwrap().size(), 21);
        assert_eq!(QrCode::encode(&[b'a'; 15]).unwrap().size(), 25);
        assert_eq!(QrCode::encode(&[b'a'; 213]).unwrap().size(), 57);
        assert!(QrCode::encode(&[b'a'; 214]).is_none());
    }

    /// Rows of a symbol, `#` for dark.
    fn rows(code: &QrCode) -> Vec<String> {
        (0..code.size())
            .map(|y| {
                (0..code.size())
                    .map(|x| if code.dark(x, y) { '#' } else { '.' })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_version_1_symbol_matches_another_encoder() {
        // "otto" in byte mode at 1-M with mask 4, as python-qrcode 8.2
        // encodes it.
        let expected = [
            "#######.##.#..#######",
            "#.....#....##.#.....#",
            "#.###.#.......#.###.#",
            "#.###.#.###...#.###.#",
            "#.###.#.#####.#.###.#",
            "#.....#.###.#.#.....#",
            "#######.#.#.#.#######",
            "........###..........",
            "#...#.###.#.######..#",
            ".##.##....#.###..##..",
            "..##.####.#.##...#.#.",
            ".#..#...#####..#...##",
            "..#.###.#.##..####...",
            "........#####..#.###.",
            "#######.##.#..####.#.",
            "#.....#......##.#....",
            "#.###.#.#...###.....#",
            "#.###.#...#.###..#.##",
            "#.###.#.....##....#..",
            "#.....#..####..#.#...",
            "#######.##.#..###.#.#",
        ];
        assert_eq!(rows(&encode_masked(b"otto", Some(4)).unwrap()), expected);
    }

    /// FNV-1a of a symbol's rows joined by newlines.
    fn fingerprint(code: &QrCode) -> u64 {
        rows(code)
            .join("\n")
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }

    #[test]
    fn larger_symbols_match_another_encoder() {
        // Uneven blocks and version information: versions 8 and 10. The
        // fingerprints are of python-qrcode 8.2's symbols for the same data
        // and mask.
        let data =
            |len: usize| -> Vec<u8> { (0..len).map(|i| ((i * 7 + 33) % 94 + 33) as u8).collect() };
        let v8 = encode_masked(&data(130), Some(3)).unwrap();
        assert_eq!(v8.size(), 49);
        assert_eq!(fingerprint(&v8), 0xc719_9711_2e20_edf1);
        let v10 = encode_masked(&data(200), Some(6)).unwrap();
        assert_eq!(v10.size(), 57);
        assert_eq!(fingerprint(&v10), 0x934b_449f_21f8_dd4d);
    }

    #[test]
    fn debug_does_not_show_the_modules() {
        let code = QrCode::encode(b"WIFI:T:WPA;S:Home;P:secret;;").unwrap();
        assert_eq!(format!("{code:?}"), "QrCode(29×29)");
    }
}
