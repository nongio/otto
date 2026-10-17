//! otto-authorize — the session's polkit authentication agent.
//!
//! When a program asks polkit for something that needs a password (`pkexec`,
//! a system service, a protected Otto setting), polkitd asks the agent
//! registered for the session. This one shows Otto's password panel, the
//! same card as the lock screen, as a dialog on a layer-shell overlay: the
//! user's name, what is being asked and by whom, the password field (and the
//! fingerprint reader, if the PAM stack uses one) and Cancel. The password
//! goes to polkit's own helper; see [`polkit`].
//!
//! The compositor starts it with `--polkit-agent` on a socketpair of its own
//! (`src/polkit_agent.rs`), so the panel keeps the keyboard while it is up.

mod dialog;
mod polkit;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    otto_kit::logging::init("info");
    otto_kit::i18n::init_from_desktop();

    if std::env::args().nth(1).as_deref() != Some(polkit::AGENT_FLAG) {
        eprintln!(
            "otto-authorize is started by Otto as `otto-authorize {}`",
            polkit::AGENT_FLAG
        );
        std::process::exit(2);
    }
    polkit::run()
}
