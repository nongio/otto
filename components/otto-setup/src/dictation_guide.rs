//! The instructions for the Dictation topic, marked up as `guide.rs`
//! describes: who does each step, commands on `$` lines, file contents
//! behind `│`.
//!
//! Words: "engine" and "speech server" (never STT, ASR or model server),
//! "the dictation in every other field" for otto-dictate.

// Rust guideline compliant 2026-02-21

use crate::dictation::{self, Dictate, Engine, Facts, Status, ENGINES, INSTALL_SH, SOURCE};

/// A step heading: `n. text`, and who does it, or that it is done.
fn step(n: usize, text: &str, you: bool, done: bool) -> String {
    let tag = match (done, you) {
        (true, _) => "#done",
        (false, true) => "#you",
        (false, false) => "#setup",
    };
    format!("{tag} {n}. {text}")
}

fn cmd(command: impl AsRef<str>) -> String {
    format!("$ {}", command.as_ref())
}

fn mark(ok: bool, what: &str, note: &str) -> String {
    let glyph = if ok { '✓' } else { '✗' };
    format!("  {glyph} {what:<18}{note}")
}

/// The command that installs what `install.sh` needs here.
fn tools_install(facts: &Facts) -> Option<&'static str> {
    Some(match facts.package_manager? {
        "pacman" => "sudo pacman -S --needed base-devel cmake git vulkan-headers shaderc spirv-headers rustup",
        "apt" => "sudo apt install build-essential cmake git libvulkan-dev glslc spirv-headers curl",
        "dnf" => "sudo dnf install gcc-c++ cmake git vulkan-headers glslc spirv-headers curl",
        "zypper" => "sudo zypper install gcc-c++ cmake git vulkan-headers shaderc spirv-headers curl",
        _ => return None,
    })
}

/// The command that installs a Vulkan driver here.
fn vulkan_install(facts: &Facts) -> Vec<String> {
    match facts.package_manager {
        Some("pacman") => vec![
            "For an AMD GPU:".into(),
            cmd("sudo pacman -S --needed vulkan-radeon"),
            "For an Intel GPU:".into(),
            cmd("sudo pacman -S --needed vulkan-intel"),
            "For an NVIDIA GPU, its driver (nvidia-utils) includes Vulkan.".into(),
        ],
        Some("apt") => vec![
            cmd("sudo apt install mesa-vulkan-drivers"),
            "For an NVIDIA GPU, its driver includes Vulkan.".into(),
        ],
        Some("dnf") => vec![
            cmd("sudo dnf install mesa-vulkan-drivers"),
            "For an NVIDIA GPU, its driver includes Vulkan.".into(),
        ],
        Some("zypper") => vec![
            cmd("sudo zypper install Mesa-vulkan-drivers"),
            "For an NVIDIA GPU, its driver includes Vulkan.".into(),
        ],
        _ => vec!["Install your GPU's Vulkan driver with your package manager.".into()],
    }
}

/// The steps that install an engine: the tools, Otto's source, `install.sh`.
/// Shared by every engine's page; `n` is the first step's number.
fn install_steps(facts: &Facts, engine: Engine, mut n: usize, out: &mut Vec<String>) -> usize {
    let missing = facts.missing_tools();
    out.push(step(n, "Install the build tools", true, missing.is_empty()));
    out.push(
        "Building the engine needs git, cmake, a C++ compiler, the Vulkan headers, glslc".into(),
    );
    out.push("and Rust's cargo (rustup.rs installs it).".into());
    if let Some(command) = tools_install(facts) {
        out.push(cmd(command));
    }
    if facts.package_manager != Some("pacman") {
        out.push("Then Rust, if cargo is not installed yet:".into());
        out.push(cmd(
            "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh",
        ));
    }
    out.push(String::new());
    n += 1;
    let has_source = facts.source.is_some();
    out.push(step(n, "Get Otto's source", true, has_source));
    match &facts.source {
        Some(dir) => out.push(format!("Found at {}.", facts.shown(dir))),
        None => {
            out.push(
                "The engines are not in Otto's packages yet; they build from its source.".into(),
            );
            out.push(cmd(format!("git clone {SOURCE} ~/otto")));
        }
    }
    out.push(String::new());
    n += 1;
    let installed = dictation::status(facts, engine).installed();
    out.push(step(
        n,
        &format!("Build {} and fetch its model", engine.name()),
        true,
        installed,
    ));
    let source = facts
        .source
        .as_deref()
        .map_or_else(|| "~/otto".to_owned(), |dir| facts.shown(dir));
    out.push(cmd(format!("{source}/{INSTALL_SH} {}", engine.id())));
    out.push("It builds the speech server, downloads the model (about 1 GB), installs".into());
    out.push("otto-dictate in ~/.local/bin and starts the engine. It takes a while.".into());
    out.push(String::new());
    n + 1
}

/// One engine: what is there, and how to have it serving.
pub fn engine(facts: &Facts, engine: Engine) -> String {
    let mut out = vec![
        format!("# {}", engine.name()),
        String::new(),
        format!("{}.", engine.about()),
        String::new(),
    ];
    let status = dictation::status(facts, engine);
    out.push(mark(
        facts.has_files(engine),
        "server and model",
        &facts.shown(&facts.data.join(engine.server())),
    ));
    let (ok, note) = match &status {
        Status::Running { enabled: true } => (true, "running, starts at login"),
        Status::Running { enabled: false } => (true, "running"),
        Status::Stopped { enabled: true } => (true, "installed, stopped"),
        Status::Stopped { enabled: false } => (true, "installed, off"),
        Status::Missing(_) => (false, "not installed"),
        Status::Unmanaged => (false, "no systemd user session"),
    };
    out.push(mark(ok, "service", &format!("{} · {note}", engine.unit())));
    out.push(String::new());
    let n = install_steps(facts, engine, 1, &mut out);
    let picked =
        facts.engine_in_file == Some(engine) && matches!(status, Status::Running { enabled: true });
    out.push(step(
        n,
        &format!("Make {} the engine and start it", engine.name()),
        false,
        picked,
    ));
    out.push(
        "Pick it on the Engine page. Apply writes it into ~/.config/otto/dictation.toml,".into(),
    );
    out.push("stops any other engine and starts this one now and at login.".into());
    out.push("Settings › Dictation switches engines later, and sets the language.".into());
    out.push(String::new());
    out.push("Then try it: Ctrl+D in the launcher, and speak.".into());
    out.join("\n")
}

/// The speech server as a whole: which engine answers.
pub fn server(facts: &Facts) -> String {
    let mut out = vec![
        "# Speech server".to_owned(),
        String::new(),
        format!(
            "Every engine serves {}; the launcher and the Ask chat send what they hear",
            dictation::SERVER
        ),
        "there. One engine runs at a time.".into(),
        String::new(),
    ];
    for engine in ENGINES {
        let note = match dictation::status(facts, engine) {
            Status::Running { enabled: true } => "running, starts at login".to_owned(),
            Status::Running { enabled: false } => "running".to_owned(),
            Status::Stopped { enabled: true } => "stopped; starts at login".to_owned(),
            Status::Stopped { enabled: false } => "installed, off".to_owned(),
            Status::Missing(missing) => format!("missing {}", missing.join(" and ")),
            Status::Unmanaged => "no systemd user session".to_owned(),
        };
        let ok = matches!(dictation::status(facts, engine), Status::Running { .. });
        out.push(mark(ok, engine.name(), &note));
    }
    out.push(mark(
        facts.answering,
        "answers",
        if facts.answering {
            "something listens at the address"
        } else {
            "nothing at the address yet"
        },
    ));
    if facts.answering && dictation::running(facts).is_none() {
        out.push(String::new());
        out.push(
            "Something answers that is not one of the engines' services: another program".into(),
        );
        out.push("on the port. Setup leaves it alone; stop it before starting an engine.".into());
    }
    out.join("\n")
}

/// The microphone: PipeWire.
pub fn microphone(facts: &Facts) -> String {
    let mut out = vec![
        "# Microphone".to_owned(),
        String::new(),
        "Dictation records from the system's default microphone through PipeWire.".into(),
        String::new(),
    ];
    match &facts.pipewire {
        Some(path) => out.push(mark(true, "pipewire", &facts.shown(path))),
        None => out.push(mark(false, "pipewire", "not installed")),
    }
    out.push(mark(
        facts.pipewire_running,
        "running",
        if facts.pipewire_running {
            "its socket is in the runtime folder"
        } else {
            "not running in this session"
        },
    ));
    out.push(String::new());
    out.push(step(1, "Install PipeWire", true, facts.pipewire.is_some()));
    if let Some(command) = match facts.package_manager {
        Some("pacman") => Some("sudo pacman -S --needed pipewire pipewire-pulse wireplumber"),
        Some("apt") => Some("sudo apt install pipewire pipewire-pulse wireplumber"),
        Some("dnf") => Some("sudo dnf install pipewire pipewire-pulseaudio wireplumber"),
        Some("zypper") => Some("sudo zypper install pipewire pipewire-pulseaudio wireplumber"),
        _ => None,
    } {
        out.push(cmd(command));
    }
    out.push("Most desktops install it already; it starts with the session.".into());
    out.push(String::new());
    out.push(step(2, "Pick the microphone", true, facts.pipewire_running));
    out.push("Settings › Sound picks the input device; dictation uses the default one.".into());
    out.join("\n")
}

/// The GPU driver: Vulkan.
pub fn vulkan(facts: &Facts) -> String {
    let mut out = vec![
        "# GPU driver".to_owned(),
        String::new(),
        "The engines run on the GPU through Vulkan. Without a Vulkan driver they".into(),
        "fail to start, or fall back to the CPU and take seconds per sentence.".into(),
        String::new(),
        mark(
            facts.vulkan,
            "Vulkan driver",
            if facts.vulkan {
                "installed"
            } else {
                "none found in the loader's folders"
            },
        ),
        String::new(),
        step(1, "Install your GPU's Vulkan driver", true, facts.vulkan),
    ];
    out.extend(vulkan_install(facts));
    out.push("Check it with vulkaninfo --summary, from the vulkan-tools package.".into());
    out.join("\n")
}

/// The build tools and the source, for installing an engine.
pub fn tools(facts: &Facts) -> String {
    let mut out = vec![
        "# Build tools".to_owned(),
        String::new(),
        "Only needed to install an engine: install.sh builds its speech server from".into(),
        "source. Once an engine is installed, they can go.".into(),
        String::new(),
    ];
    for (name, path) in &facts.tools {
        match path {
            Some(path) => out.push(mark(true, name, &facts.shown(path))),
            None => out.push(mark(false, name, "not found")),
        }
    }
    match &facts.source {
        Some(dir) => out.push(mark(true, "Otto's source", &facts.shown(dir))),
        None => out.push(mark(false, "Otto's source", "no checkout found")),
    }
    out.push(String::new());
    install_steps(facts, Engine::Parakeet, 1, &mut out);
    out.join("\n")
}

/// otto-dictate: dictation in every other field.
pub fn dictate(facts: &Facts) -> String {
    let state = dictation::dictate(facts);
    let mut out = vec![
        "# Dictation in every other field".to_owned(),
        String::new(),
        "Ctrl+D dictates in the launcher and the Ask chat without more. For every".into(),
        "other text field there is otto-dictate: it runs with the session, and a".into(),
        "key of your choice starts and stops it in the field that has the focus.".into(),
        String::new(),
    ];
    match &facts.dictate {
        Some(path) => out.push(mark(true, "otto-dictate", &facts.shown(path))),
        None => out.push(mark(false, "otto-dictate", "not installed")),
    }
    out.push(mark(
        state == Dictate::On,
        "at login",
        match state {
            Dictate::On => "starts with the session",
            Dictate::Off => "not yet",
            Dictate::NotInstalled => "needs otto-dictate",
        },
    ));
    match &facts.shortcut {
        Some(key) => out.push(mark(
            true,
            "shortcut",
            &format!("{key} in ~/.config/otto/config.toml"),
        )),
        None => out.push(mark(false, "shortcut", "none bound to otto-dictate")),
    }
    out.push(String::new());
    out.push(step(
        1,
        "Install otto-dictate",
        true,
        facts.dictate.is_some(),
    ));
    out.push("install.sh puts it in ~/.local/bin when it installs an engine (Enter on an".into());
    out.push("engine shows how). From a checkout of Otto's source on its own:".into());
    let source = facts
        .source
        .as_deref()
        .map_or_else(|| "~/otto".to_owned(), |dir| facts.shown(dir));
    out.push(cmd(format!(
        "cargo install --path {source}/components/otto-dictate --root ~/.local"
    )));
    out.push(String::new());
    out.push(step(2, "Start it at login", false, state == Dictate::On));
    out.push(
        "Tick it on the Requirements page. Apply writes ~/.config/autostart/otto-dictate.desktop."
            .into(),
    );
    out.push("Otto reads that folder with xdg_autostart = true in its config.toml.".into());
    out.push("Settings › Dictation turns it off again.".into());
    out.push(String::new());
    out.push(step(3, "Bind a key to it", true, facts.shortcut.is_some()));
    out.push("In ~/.config/otto/config.toml, under [keyboard_shortcuts]; pick any key:".into());
    out.push("│ [keyboard_shortcuts]".into());
    let exec = facts
        .dictate
        .as_deref()
        .map_or_else(|| "otto-dictate".to_owned(), |p| p.display().to_string());
    out.push(format!(
        "│ \"Logo+D\" = {{ run = {{ cmd = \"{exec}\", args = [\"toggle\"] }} }}"
    ));
    out.push(
        "Otto reads config.toml at login, so it works from the next session. Press the".into(),
    );
    out.push("key in a text field, speak, press it again: the words go in.".into());
    out.join("\n")
}
