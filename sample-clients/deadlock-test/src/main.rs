//! X11 client that reproduces the Unity/Proton game deadlock on Otto compositor.
//!
//! Mimics Cuphead's startup pattern:
//! 1. Creates a window with WM_HINTS: iconic state + no input focus
//! 2. Maps the window
//! 3. Waits for WM_TAKE_FOCUS + _NET_ACTIVE_WINDOW
//! 4. On Plasma: activated within 1-2 seconds
//! 5. On Otto:   deadlocked, focus flaps, _NET_ACTIVE_WINDOW never set
//!
//! Build:  cargo build -p deadlock-test
//! Run:    cargo run -p deadlock-test

use std::time::Instant;

use x11rb::atom_manager;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

atom_manager! {
    pub AtomCollection: AtomCollectionCookie {
        WM_PROTOCOLS,
        WM_TAKE_FOCUS,
        WM_DELETE_WINDOW,
        _NET_ACTIVE_WINDOW,
        _NET_WM_STATE,
        _NET_WM_STATE_FOCUSED,
        _NET_WM_STATE_FULLSCREEN,
        _NET_WM_STATE_HIDDEN,
        _NET_WM_BYPASS_COMPOSITOR,
        _NET_WM_NAME,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (conn, screen_num) = RustConnection::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let atoms = AtomCollection::new(&conn)?.reply()?;

    let win = conn.generate_id()?;
    let black = conn.generate_id()?;
    let white = conn.generate_id()?;

    // Create graphics contexts
    conn.create_gc(
        black,
        screen.root,
        &CreateGCAux::new().foreground(screen.black_pixel),
    )?;
    conn.create_gc(
        white,
        screen.root,
        &CreateGCAux::new().foreground(screen.white_pixel),
    )?;

    // Create window
    let depth = screen.root_depth;
    conn.create_window(
        depth,
        win,
        screen.root,
        100, // x
        100, // y
        800, // width
        600, // height
        1,   // border
        WindowClass::INPUT_OUTPUT,
        0, // visual
        &CreateWindowAux::new()
            .background_pixel(screen.black_pixel)
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::FOCUS_CHANGE
                    | EventMask::STRUCTURE_NOTIFY
                    | EventMask::PROPERTY_CHANGE
                    | EventMask::KEY_PRESS,
            ),
    )?;

    // ---- Set WM_HINTS: iconic state + no input focus ----
    // This is exactly what Cuphead/Unity does.
    let hints_data: [u32; 9] = {
        // XWMHints structure (order: flags, input, initial_state, icon_pixmap,
        //                        icon_window, icon_x, icon_y, icon_mask, window_group)
        let flags: u32 = 0x0003; // StateHint | InputHint
        let input: u32 = 0; // False = client doesn't accept input focus
        let initial_state: u32 = 3; // IconicState
        let icon_pixmap: u32 = 0;
        let icon_window: u32 = 0;
        let icon_x: u32 = 0;
        let icon_y: u32 = 0;
        let icon_mask: u32 = 0;
        let window_group: u32 = 0;

        [
            flags,
            input,
            initial_state,
            icon_pixmap,
            icon_window,
            icon_x,
            icon_y,
            icon_mask,
            window_group,
        ]
    };
    conn.change_property32(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_HINTS,
        AtomEnum::WM_HINTS,
        &hints_data,
    )?;

    // Set window name
    conn.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"deadlock-test",
    )?;

    // Set _NET_WM_NAME (need to intern UTF8_STRING atom)
    let utf8_string = conn.intern_atom(false, b"UTF8_STRING")?.reply()?.atom;
    conn.change_property8(
        PropMode::REPLACE,
        win,
        atoms._NET_WM_NAME,
        utf8_string,
        b"deadlock-test",
    )?;

    // Set WM_PROTOCOLS: WM_TAKE_FOCUS, WM_DELETE_WINDOW
    let protocols = [atoms.WM_TAKE_FOCUS, atoms.WM_DELETE_WINDOW];
    conn.change_property32(
        PropMode::REPLACE,
        win,
        atoms.WM_PROTOCOLS,
        AtomEnum::ATOM,
        &protocols,
    )?;

    // Map the window first — WM only processes client messages for mapped windows
    conn.map_window(win)?;
    conn.flush()?;

    // Now request _NET_WM_BYPASS_COMPOSITOR (like Cuphead/Unity fullscreen games)
    conn.change_property32(
        PropMode::REPLACE,
        win,
        atoms._NET_WM_BYPASS_COMPOSITOR,
        AtomEnum::CARDINAL,
        &[1],
    )?;

    // Send _NET_WM_STATE client message to request fullscreen.
    // This is what triggers Plasma to activate the window despite iconic hints.
    let wm_state = conn.intern_atom(false, b"_NET_WM_STATE")?.reply()?.atom;
    let mut event = [0u8; 32];
    event[0] = 33; // ClientMessage
    event[1] = 32; // format
                   // sequence (bytes 2-3): leave 0
                   // window (bytes 4-7)
    event[4..8].copy_from_slice(&win.to_ne_bytes());
    // message_type (bytes 8-11)
    event[8..12].copy_from_slice(&wm_state.to_ne_bytes());
    // data[0] = _NET_WM_STATE_ADD (1)
    event[12..16].copy_from_slice(&1u32.to_ne_bytes());
    // data[1] = _NET_WM_STATE_FULLSCREEN
    event[16..20].copy_from_slice(&atoms._NET_WM_STATE_FULLSCREEN.to_ne_bytes());
    // data[2] = 0 (second property, none)
    // data[3] = 1 (source indication: 1 = application)
    event[24..28].copy_from_slice(&1u32.to_ne_bytes());
    conn.send_event(
        false,
        screen.root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    )?;

    eprintln!("Window mapped (ID: 0x{:x}). Waiting for activation...", win);

    let start = Instant::now();
    let mut has_focus = false;
    let mut activated = false;
    let mut first_focus_time: Option<Instant> = None;

    loop {
        let elapsed = start.elapsed().as_secs();
        if elapsed >= 10 {
            eprintln!();
            eprintln!("*** DEADLOCK after {} seconds ***", elapsed);
            eprintln!("   has_focus={} activated={}", has_focus, activated);
            eprintln!("   This is the CUPHEAD BUG on Otto.");
            break;
        }

        // Poll _NET_ACTIVE_WINDOW on root
        if let Ok(reply) = conn
            .get_property(
                false,
                screen.root,
                atoms._NET_ACTIVE_WINDOW,
                AtomEnum::WINDOW,
                0,
                1,
            )?
            .reply()
        {
            let active = reply.value32().and_then(|mut v| v.next());
            if active == Some(win) && !activated {
                activated = true;
                eprintln!("[+] _NET_ACTIVE_WINDOW set to us! Game should start now.");
            }
        }

        // Poll _NET_WM_STATE on our window
        if let Ok(reply) = conn
            .get_property(false, win, atoms._NET_WM_STATE, AtomEnum::ATOM, 0, 64)?
            .reply()
        {
            if reply.format == 32 && reply.length > 0 {
                eprint!("   _NET_WM_STATE:");
                if let Some(atoms_iter) = reply.value32() {
                    for atom in atoms_iter {
                        let name = conn.get_atom_name(atom);
                        if let Ok(reply) = name?.reply() {
                            eprint!(" {}", String::from_utf8_lossy(&reply.name));
                        }
                    }
                }
                eprintln!();
            }
        }

        // Process events
        while let Some(event) = conn.poll_for_event()? {
            match event {
                Event::FocusIn(ev) => {
                    has_focus = true;
                    if first_focus_time.is_none() {
                        first_focus_time = Some(Instant::now());
                    }
                    eprintln!("[+] FocusIn (mode={:?}, detail={:?})", ev.mode, ev.detail);

                    // Debug: immediately read _NET_ACTIVE_WINDOW after FocusIn
                    if let Ok(reply) = conn
                        .get_property(
                            false,
                            screen.root,
                            atoms._NET_ACTIVE_WINDOW,
                            AtomEnum::WINDOW,
                            0,
                            1,
                        )?
                        .reply()
                    {
                        let active = reply.value32().and_then(|mut v| v.next());
                        eprintln!(
                            "   Debug: _NET_ACTIVE_WINDOW on root = 0x{:x?} (format={}, len={})",
                            active, reply.format, reply.length
                        );
                    }
                }
                Event::FocusOut(ev) => {
                    has_focus = false;
                    eprintln!("[-] FocusOut (mode={:?}, detail={:?})", ev.mode, ev.detail);
                }
                Event::MapNotify(_) => {
                    eprintln!("[.] MapNotify");
                }
                Event::ClientMessage(ev) => {
                    if ev.type_ == atoms.WM_PROTOCOLS && ev.format == 32 {
                        let protocol = ev.data.as_data32()[0];
                        if protocol == atoms.WM_TAKE_FOCUS {
                            let timestamp = ev.data.as_data32()[1];
                            eprintln!(
                                "[+] WM_TAKE_FOCUS received (ts={}). Taking focus.",
                                timestamp
                            );

                            // Accept focus
                            conn.set_input_focus(InputFocus::PARENT, win, timestamp)?;
                            has_focus = true;
                        }
                    }
                }
                Event::Expose(_) => {
                    // Redraw: just clear to black and draw a white rectangle
                    conn.poly_fill_rectangle(
                        win,
                        black,
                        &[Rectangle {
                            x: 0,
                            y: 0,
                            width: 800,
                            height: 600,
                        }],
                    )?;
                    conn.poly_fill_rectangle(
                        win,
                        white,
                        &[Rectangle {
                            x: 50,
                            y: 50,
                            width: 200,
                            height: 100,
                        }],
                    )?;
                    conn.flush()?;
                }
                Event::KeyPress(_) => {
                    eprintln!("[.] KeyPress — exiting.");
                    return Ok(());
                }
                _ => {}
            }
        }

        // Success condition: got focus + activation
        if has_focus && activated {
            eprintln!();
            eprintln!("*** SUCCESS after {} seconds ***", elapsed);
            eprintln!(
                "   Focus received at t={:.1}s, activated. Otto fix works.",
                first_focus_time
                    .map(|t| t.duration_since(start).as_secs_f64())
                    .unwrap_or(0.0)
            );

            // Short render loop to prove we're alive
            eprintln!("   Simulating render loop (5 frames, press any key to exit)...");
            for frame in 1..=5 {
                // Check for keypress to exit early
                while let Some(event) = conn.poll_for_event()? {
                    if let Event::KeyPress(_) = event {
                        eprintln!("   Exiting on keypress.");
                        return Ok(());
                    }
                }

                conn.poly_fill_rectangle(
                    win,
                    black,
                    &[Rectangle {
                        x: 0,
                        y: 0,
                        width: 800,
                        height: 600,
                    }],
                )?;
                conn.poly_fill_rectangle(
                    win,
                    white,
                    &[Rectangle {
                        x: 50 + frame as i16 * 30,
                        y: 50 + frame as i16 * 30,
                        width: 200,
                        height: 100,
                    }],
                )?;
                conn.flush()?;
                eprintln!("   Frame {} rendered", frame);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            return Ok(());
        }

        // Prevent busy-waiting
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    eprintln!();
    eprintln!("=== Final window state ===");

    // Check map state
    if let Ok(reply) = conn.get_window_attributes(win)?.reply() {
        let state = match reply.map_state {
            MapState::UNMAPPED => "Unmapped",
            MapState::UNVIEWABLE => "Unviewable",
            MapState::VIEWABLE => "Viewable",
            _ => "Unknown",
        };
        eprintln!("Map state: {}", state);
    }

    // Print WM_HINTS
    if let Ok(reply) = conn
        .get_property(false, win, AtomEnum::WM_HINTS, AtomEnum::WM_HINTS, 0, 9)?
        .reply()
    {
        if let Some(values) = reply.value32() {
            let vals: Vec<u32> = values.collect();
            eprintln!(
                "WM_HINTS: flags=0x{:x} input={} initial_state={}",
                vals.get(0).unwrap_or(&0),
                vals.get(1).unwrap_or(&0),
                vals.get(2).unwrap_or(&0)
            );
        }
    }

    eprintln!("Exit: DEADLOCK (exit code 1)");
    std::process::exit(1);
}
