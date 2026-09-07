//! Telling clients where the text cursor is.
//!
//! Applications report their caret through `zwp_text_input_v3`, in the
//! coordinates of their own surface, and the compositor normally passes it
//! straight to the input method so a candidate list can sit under the word
//! being typed. `otto-text-cursor-v1` offers the same position to an ordinary
//! client, so something that acts on the text — the emoji picker — can appear
//! beside the caret instead of in the middle of the screen.
//!
//! Two things make this awkward, and both are why the position is *retained*
//! rather than read live:
//!
//! - A client asking has usually just taken the keyboard for itself, which
//!   deactivates the text input it wants to sit beside. The answer has to
//!   survive the questioner arriving.
//! - Reporting a caret is optional, and plenty of applications never do — most
//!   terminals, and everything under Xwayland. `unavailable` is a normal
//!   answer, not an error, and clients are expected to have a fallback.

pub mod handlers;
pub mod protocol;

pub use handlers::TextCursorState;
pub use protocol::{OttoTextCursorManagerV1, OttoTextCursorV1};
