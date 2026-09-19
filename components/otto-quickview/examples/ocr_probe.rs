//! Decode a picture with text recognition through the sandboxed worker and
//! print the words. `cargo run -p otto-quickview --example ocr_probe -- <path> [recogniser command]`
fn main() {
    otto_quickview::run_worker_if_requested();
    let path = std::env::args().nth(1).expect("path");
    let recogniser = std::env::args().nth(2).unwrap_or_default();
    let request = otto_quickview::decode::Request {
        ocr: true,
        languages: otto_quickview::ocr::languages(),
        recogniser,
        ..Default::default()
    };
    let started = std::time::Instant::now();
    let preview = otto_quickview::decode_path(std::path::Path::new(&path), &request);
    println!(
        "recogniser={} languages={} took={:?}",
        request.recogniser_command(),
        request.languages,
        started.elapsed()
    );
    match preview {
        otto_quickview::Preview::Pixels { pixels, .. } => {
            println!(
                "{}x{} words={}",
                pixels.width,
                pixels.height,
                pixels.words.len()
            );
            for w in pixels.words.iter().take(20) {
                println!(
                    "{:>4} {:>4} {:>4} {:>4} {:>3} {}",
                    w.left, w.top, w.width, w.height, w.confidence, w.text
                );
            }
        }
        other => println!("{other:?}"),
    }
}
