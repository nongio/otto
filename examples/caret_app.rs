//! A window that reports a text cursor at a known spot, so a client which
//! places itself at the caret can be checked against something visible.
//!
//! ```sh
//! cargo run --example caret_app -- wayland-2 120 300
//! ```

fn main() {
    let mut args = std::env::args().skip(1);
    let socket = args.next().expect("usage: caret_app <socket> [x] [y]");
    let x: i32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(120);
    let y: i32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);
    std::env::set_var("WAYLAND_DISPLAY", &socket);

    let mut client = otto_kit::testing::TestClient::connect(&socket).expect("connect");
    let _window = client.create_toplevel("caret-app", 700, 500);
    let _ = client.roundtrip();
    std::thread::sleep(std::time::Duration::from_millis(400));
    println!("reported caret: {}", client.set_text_cursor(x, y, 2, 20));
    loop {
        let _ = client.roundtrip();
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}
