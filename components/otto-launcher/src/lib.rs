//! The launcher's parts, as a library so the preview example — and tests —
//! can build the same scene the binary does without a compositor.

pub mod apps;
pub mod calc;
pub mod drafts;
pub mod source;
pub mod view;
pub mod windows;

pub use apps::Apps;
pub use calc::Calculator;
pub use source::Source;
pub use view::{Palette, CARD_W, FIELD_H, MAX_CARD_H, RADIUS};
