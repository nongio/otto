//! Login (greeter) mode.
//!
//! When Otto is started with `--login` it acts as the host compositor for a
//! greeter client — the role `cage` plays for `gtkgreet`, or `weston
//! --shell=fullscreen-shell` plays for SDDM's Qt greeter.
//!
//! In this mode Otto deliberately suppresses everything that belongs to a user
//! session:
//!
//! * no dock, no app switcher, no expose / workspace selector
//! * only the primary output is driven; other connectors are ignored
//! * exactly one client is launched (the greeter), and its first toplevel is
//!   forced fullscreen
//!
//! Otto never touches PAM. Authentication is delegated to [greetd], which runs
//! as root, owns the VT, and hands Otto's greeter client a socket on
//! `GREETD_SOCK`. The greeter speaks greetd's JSON IPC and greetd performs the
//! `pam_open_session` and execs the user's session.
//!
//! [greetd]: https://sr.ht/~kennylevinsen/greetd/

use std::sync::atomic::{AtomicBool, Ordering};

static LOGIN_MODE: AtomicBool = AtomicBool::new(false);

/// Enable login mode. Called once from `main` before any state is built.
pub fn set_login_mode(enabled: bool) {
    LOGIN_MODE.store(enabled, Ordering::Relaxed);
}

/// Whether Otto is running as a greeter host rather than a user session.
#[inline]
pub fn is_login_mode() -> bool {
    LOGIN_MODE.load(Ordering::Relaxed)
}

/// The greeter to launch, as `(command, args)`.
///
/// `$OTTO_GREETER_COMMAND` overrides the configured command and is parsed as a
/// whitespace-separated argv, so an uninstalled build can be tested with
/// `OTTO_GREETER_COMMAND=target/release/otto-greeter otto --winit --login`.
pub fn greeter_command() -> (String, Vec<String>) {
    if let Ok(override_cmd) = std::env::var("OTTO_GREETER_COMMAND") {
        let mut argv = override_cmd.split_whitespace().map(str::to_string);
        if let Some(cmd) = argv.next() {
            return (cmd, argv.collect());
        }
    }
    crate::config::Config::with(|c| {
        (
            c.login.greeter_command.clone(),
            c.login.greeter_args.clone(),
        )
    })
}
