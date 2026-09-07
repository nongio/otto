//! Type text into the focused window through the picker's virtual-keyboard
//! path, without the picker. For checking how a client decodes what the
//! compositor forwards:
//!
//! ```sh
//! wev &                       # or any window that shows what it receives
//! cargo run -p otto-emoji --example type -- "👋🏽❤️"
//! ```

fn main() {
    let text: Vec<String> = std::env::args().skip(1).collect();
    let text = text.join(" ");
    if let Err(err) = otto_emoji::typing::type_text(&text) {
        eprintln!("could not type: {err}");
        std::process::exit(1);
    }
}
