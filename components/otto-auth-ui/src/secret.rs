//! What has been typed into the panel's field, kept so it can be forgotten.
//!
//! A password lives in a few places on its way to PAM or greetd: the field's
//! buffer, the answer handed over on Enter, and whatever that answer is copied
//! into. Every one of them is overwritten before its memory goes back to the
//! allocator — [`zeroize`] makes the writes stick, where a plain `clear()`
//! only moves the length.
//!
//! The buffer is reserved in full up front and never grows past it. A `String`
//! that grows reallocates, and the old allocation, password and all, is freed
//! with nothing to wipe it: so input that would not fit is refused instead.

use zeroize::{Zeroize, Zeroizing};

/// The field's buffer: a string that never reallocates and is wiped whenever
/// anything leaves it — a character to Backspace, the whole of it to Escape,
/// or the answer to Enter.
pub struct SecretInput(Zeroizing<String>);

impl SecretInput {
    /// Bytes reserved for the buffer. Far more than any password or username
    /// anyone types; what does not fit is dropped rather than reallocated.
    pub const CAPACITY: usize = 1024;

    pub fn new() -> Self {
        Self(Zeroizing::new(String::with_capacity(Self::CAPACITY)))
    }

    /// Append `text`, or nothing if it would not fit. Returns whether it went
    /// in.
    pub fn push_str(&mut self, text: &str) -> bool {
        if self.0.len() + text.len() > self.0.capacity() {
            return false;
        }
        self.0.push_str(text);
        true
    }

    /// Append one character, or nothing if it would not fit.
    pub fn push(&mut self, c: char) -> bool {
        self.push_str(c.encode_utf8(&mut [0; 4]))
    }

    /// Remove the last character, overwriting its bytes.
    pub fn pop(&mut self) -> bool {
        let Some(c) = self.0.chars().next_back() else {
            return false;
        };
        let end = self.0.len() - c.len_utf8();
        // SAFETY: the bytes after `end` are one whole character, and zero
        // bytes are valid UTF-8, so the string is a string throughout; it is
        // then cut at `end`, a character boundary.
        unsafe { self.0.as_mut_vec()[end..].zeroize() };
        self.0.truncate(end);
        true
    }

    /// Empty the buffer, overwriting everything it held. The allocation is
    /// kept, so the next thing typed lands in the same, wiped, memory.
    pub fn clear(&mut self) {
        self.0.zeroize();
    }

    /// Replace the contents with `text` (truncated to what fits).
    pub fn set(&mut self, text: &str) {
        self.clear();
        for c in text.chars() {
            if !self.push(c) {
                break;
            }
        }
    }

    /// Hand over what was typed, leaving the field empty. The answer is wiped
    /// when whoever it is handed to drops it.
    pub fn take(&mut self) -> Zeroizing<String> {
        std::mem::take(self).0
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How many characters the field shows — all a masked field is told.
    pub fn chars(&self) -> usize {
        self.0.chars().count()
    }
}

impl Default for SecretInput {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for SecretInput {
    fn from(text: &str) -> Self {
        let mut input = Self::new();
        input.set(text);
        input
    }
}

/// Only the length, so a stray `{:?}` cannot put a password in the log.
impl std::fmt::Debug for SecretInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretInput({} bytes)", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Editing never reallocates — a reallocation would free a copy of the
    /// password nobody can wipe — and what was removed does not linger past
    /// the end of the string.
    #[test]
    fn edits_stay_in_one_wiped_allocation() {
        let mut input = SecretInput::new();
        let start = input.0.as_ptr();

        assert!(input.push_str("hunter"));
        assert!(input.push('é'));
        assert_eq!(input.as_str(), "hunteré");
        assert_eq!(input.chars(), 7);

        assert!(input.pop());
        assert_eq!(input.as_str(), "hunter");
        let spare = |input: &SecretInput| {
            // SAFETY: within the allocation; the bytes were written by the
            // pushes above (or zeroed since), so they are initialised.
            unsafe { std::slice::from_raw_parts(input.0.as_ptr(), 16) }.to_vec()
        };
        assert_eq!(&spare(&input)[6..8], &[0, 0], "the popped 'é' is wiped");

        input.clear();
        assert!(input.is_empty());
        assert!(spare(&input).iter().all(|&b| b == 0), "cleared is wiped");
        assert_eq!(input.0.as_ptr(), start, "never reallocated");

        let too_long = "x".repeat(SecretInput::CAPACITY + 1);
        assert!(!input.push_str(&too_long), "refused rather than grown");
        assert_eq!(input.0.as_ptr(), start);
    }

    /// Enter hands the answer over and leaves an empty, usable field.
    #[test]
    fn take_leaves_an_empty_field() {
        let mut input = SecretInput::from("hunter2");
        let answer = input.take();
        assert_eq!(answer.as_str(), "hunter2");
        assert!(input.is_empty());
        assert!(input.push_str("again"));
        assert_eq!(format!("{input:?}"), "SecretInput(5 bytes)");
    }
}
