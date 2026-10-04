//! The account pane: who is logged in — their picture, their name, their
//! password.
//!
//! None of it is an Otto setting. The name and picture belong to the system's
//! user database, and are written through AccountsService
//! (`org.freedesktop.Accounts`) because that is where the greeter and the lock
//! screen read them back from (`otto-auth-ui`'s `user.rs`: the AccountsService
//! icon, then `~/.face`; the real name from `/etc/passwd`, which
//! AccountsService keeps). Both calls are ones a user may make on their own
//! account without a password under the stock polkit rules.
//!
//! The password is changed by `passwd`, the way every desktop that does not
//! ship its own PAM helper does it: the program runs on a thread of its own and
//! is answered prompt by prompt over a pipe. The passwords go through its
//! standard input, never its arguments, and are dropped from this process as
//! soon as the attempt is over, whichever way it went.
//!
//! The rows carry identifiers in an `account.` namespace the compositor does
//! not serve, so `main.rs` hands their edits here (see [`owns`]).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::model::{group, untitled, Control, Pane, Row};

pub const PICTURE_ID: &str = "account.picture";
const NAME_ID: &str = "account.real_name";
const CURRENT_ID: &str = "account.password.current";
const NEW_ID: &str = "account.password.new";
const CONFIRM_ID: &str = "account.password.confirm";

const BUS_NAME: &str = "org.freedesktop.Accounts";
const USER_INTERFACE: &str = "org.freedesktop.Accounts.User";

/// The side of the square a chosen picture is cut to. Large enough for the
/// greeter's 96pt avatar at 2x with room to spare, small enough that
/// AccountsService — which refuses icons over a megabyte — takes it.
const PICTURE_PX: i32 = 256;

/// What the pane shows and what the user has typed into it so far.
#[derive(Default)]
struct State {
    /// Login name.
    user: String,
    real_name: String,
    /// The picture's path, empty when there is none.
    picture: String,
    administrator: bool,
    /// The AccountsService object for this user. `None` until it has been
    /// found, and for good where the service is not running — the name and
    /// picture are then shown but cannot be changed from here.
    object: Option<String>,
    /// Whether the lookup has finished, so "not running" is not claimed of a
    /// service that simply has not answered yet.
    looked_up: bool,
    /// Why the last change to the name or picture did not take.
    profile_error: Option<String>,
    current: String,
    new: String,
    confirm: String,
    password: PasswordStatus,
}

#[derive(Default, Clone, PartialEq, Debug)]
enum PasswordStatus {
    #[default]
    Idle,
    Changing,
    Changed,
    Failed(String),
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        let state = Mutex::new(local_profile());
        // The first read off the bus happens away from the main thread; the
        // pane draws what `/etc/passwd` says until it answers.
        spawn("account-lookup", refresh);
        state
    })
}

static DIRTY: AtomicBool = AtomicBool::new(false);

/// Whether the pane changed since the last call; `main.rs` polls this to
/// repaint.
pub fn take_dirty() -> bool {
    DIRTY.swap(false, Ordering::Relaxed)
}

fn changed() {
    DIRTY.store(true, Ordering::Relaxed);
    otto_kit::AppContext::request_wakeup();
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(err) = std::thread::Builder::new().name(name.into()).spawn(work) {
        eprintln!("account: could not start {name} ({err})");
    }
}

pub fn build() -> Pane {
    let state = state().lock().unwrap();
    let editable = state.object.is_some();

    let picture = Row::new(
        otto_kit::t!("settings-account-picture"),
        if editable {
            Control::File(state.picture.clone())
        } else {
            Control::Value(String::new())
        },
    )
    .detail(otto_kit::t!("settings-account-picture-detail"))
    .id(PICTURE_ID);

    let name = Row::new(
        otto_kit::t!("settings-account-full-name"),
        if editable {
            Control::Text(state.real_name.clone())
        } else {
            Control::Value(state.real_name.clone())
        },
    )
    .id(NAME_ID);
    let name = match (&state.profile_error, editable, state.looked_up) {
        (Some(error), _, _) => name.detail(error.clone()),
        (None, false, true) => name.detail(otto_kit::t!("settings-account-no-accountsservice")),
        _ => name,
    };

    let kind = if state.administrator {
        otto_kit::t!("settings-account-type-administrator")
    } else {
        otto_kit::t!("settings-account-type-standard")
    };

    let changing = state.password == PasswordStatus::Changing;
    let status = match &state.password {
        PasswordStatus::Idle => otto_kit::t_owned!("settings-account-password-detail"),
        PasswordStatus::Changing => otto_kit::t_owned!("settings-account-password-changing"),
        PasswordStatus::Changed => otto_kit::t_owned!("settings-account-password-changed"),
        PasswordStatus::Failed(why) => why.clone(),
    };

    Pane {
        name: otto_kit::t!("settings-pane-account"),
        icon: "person",
        intro: None,
        groups: vec![
            untitled(vec![
                picture,
                name,
                Row::new(
                    otto_kit::t!("settings-account-name"),
                    Control::Value(state.user.clone()),
                ),
                Row::new(
                    otto_kit::t!("settings-account-type"),
                    Control::Value(kind.to_string()),
                ),
            ]),
            group(
                otto_kit::t!("settings-group-password"),
                vec![
                    secret_row(
                        otto_kit::t!("settings-account-current-password"),
                        &state.current,
                        CURRENT_ID,
                    ),
                    secret_row(
                        otto_kit::t!("settings-account-new-password"),
                        &state.new,
                        NEW_ID,
                    ),
                    secret_row(
                        otto_kit::t!("settings-account-confirm-password"),
                        &state.confirm,
                        CONFIRM_ID,
                    ),
                    Row::new(change_label(), Control::Button(change_buttons()))
                        .detail(status)
                        .inactive(changing),
                ],
            ),
        ],
    }
}

fn secret_row(label: &'static str, value: &str, id: &'static str) -> Row {
    Row::new(label, Control::Text(value.to_string()))
        .id(id)
        .secret(true)
}

/// The Change Password row's label, which is also its button's — the row does
/// one thing and says so once on each side.
fn change_label() -> &'static str {
    otto_kit::t!("settings-account-change-password")
}

fn change_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![otto_kit::t!("settings-account-change")])
}

/// Whether `id` is one of this pane's rows rather than a compositor setting.
pub fn owns(id: &str) -> bool {
    id.starts_with("account.")
}

/// Whether a field's contents must be masked: the three password fields.
pub fn is_secret(id: &str) -> bool {
    matches!(id, CURRENT_ID | NEW_ID | CONFIRM_ID)
}

/// A field on this pane was committed.
pub fn commit_text(id: &str, text: &str) {
    let mut state = state().lock().unwrap();
    match id {
        CURRENT_ID => state.current = text.to_string(),
        NEW_ID => state.new = text.to_string(),
        CONFIRM_ID => state.confirm = text.to_string(),
        NAME_ID => {
            let name = text.trim().to_string();
            if name == state.real_name {
                return;
            }
            let Some(object) = state.object.clone() else {
                return;
            };
            // Shown at once; the refresh after the call puts back whatever
            // the service actually kept.
            state.real_name = name.clone();
            drop(state);
            spawn("account-name", move || {
                let outcome = call(&object, "SetRealName", &(name.as_str(),));
                settle(outcome);
            });
            return;
        }
        _ => return,
    }
    // Typing again after an attempt starts a new one.
    if !matches!(state.password, PasswordStatus::Changing) {
        state.password = PasswordStatus::Idle;
    }
}

/// The picture row's file was chosen, or removed (an empty path). Returns
/// whether `id` was this pane's, so `main::apply` can stop there.
pub fn apply(id: &str, value: &crate::settings_client::Value) -> bool {
    if id != PICTURE_ID {
        return owns(id);
    }
    let crate::settings_client::Value::Text(path) = value else {
        return true;
    };
    let Some(object) = state().lock().unwrap().object.clone() else {
        return true;
    };
    let path = path.clone();
    spawn("account-picture", move || {
        let outcome = if path.is_empty() {
            remove_face();
            call(&object, "SetIconFile", &("",))
        } else {
            match write_face(Path::new(&path)) {
                Ok(face) => call(&object, "SetIconFile", &(face.to_string_lossy().as_ref(),)),
                Err(why) => Err(why),
            }
        };
        settle(outcome);
    });
    true
}

/// Record how a change to the name or picture went, and re-read both.
fn settle(outcome: Result<(), String>) {
    state().lock().unwrap().profile_error = outcome.err();
    refresh();
}

/// A press on this pane's push buttons.
pub fn press(row: &str, _button: &str) {
    if row != change_label() {
        return;
    }
    let mut held = state().lock().unwrap();
    if held.password == PasswordStatus::Changing {
        return;
    }
    let problem = if held.current.is_empty() || held.new.is_empty() {
        Some(otto_kit::t!("settings-account-password-missing"))
    } else if held.new != held.confirm {
        Some(otto_kit::t!("settings-account-password-mismatch"))
    } else if held.new == held.current {
        Some(otto_kit::t!("settings-account-password-same"))
    } else {
        None
    };
    if let Some(problem) = problem {
        held.password = PasswordStatus::Failed(problem.to_string());
        drop(held);
        changed();
        return;
    }

    // Taken out of the pane as the attempt starts: they live on only in the
    // thread that answers `passwd`, and go when it does.
    let current = std::mem::take(&mut held.current);
    let new = std::mem::take(&mut held.new);
    held.confirm.clear();
    held.password = PasswordStatus::Changing;
    drop(held);
    changed();

    spawn("account-password", move || {
        let status = match change_password("passwd", &current, &new) {
            Ok(()) => PasswordStatus::Changed,
            Err(PasswdError::WrongCurrent) => PasswordStatus::Failed(
                otto_kit::t!("settings-account-password-wrong-current").to_string(),
            ),
            Err(PasswdError::Refused(why)) => PasswordStatus::Failed(why),
        };
        state().lock().unwrap().password = status;
        changed();
    });
}

/// What `passwd` said no to.
#[derive(Debug, PartialEq)]
enum PasswdError {
    /// It stopped after the current password, before asking for a new one.
    WrongCurrent,
    /// Anything else, in `passwd`'s (or PAM's) own words.
    Refused(String),
}

/// Run `program` — `passwd`, or a stand-in under test — and answer its
/// prompts: the current password, then the new one twice.
///
/// A prompt is recognised by shape, not wording: output that stops, without a
/// line break, after a colon. That holds in every language PAM is translated
/// into, so the user's locale is left alone and a refusal comes back in it.
/// A fourth prompt is a password-quality module asking again after refusing
/// the new one; it is not answered, since the answer would be the same.
fn change_password(program: &str, current: &str, new: &str) -> Result<(), PasswdError> {
    let failed = |err: std::io::Error| PasswdError::Refused(err.to_string());
    let (mut output, writer) = std::io::pipe().map_err(failed)?;
    let mut child = {
        let mut command = Command::new(program);
        command
            .stdin(Stdio::piped())
            .stdout(writer.try_clone().map_err(failed)?)
            .stderr(writer);
        // Spawned inside a block so the command — and the pipe's write ends
        // it holds — is gone once the child has them, or reading would never
        // see the end of the output.
        command.spawn().map_err(failed)?
    };
    let mut input = child.stdin.take().expect("stdin is piped");

    let answers = [current, new, new];
    let mut answered = 0;
    // What it has said since the last answer, which is where a reason for
    // refusing ends up.
    let mut said = String::new();
    let mut pending = String::new();
    let mut buffer = [0u8; 512];
    loop {
        let read = match output.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let text = String::from_utf8_lossy(&buffer[..read]);
        said.push_str(&text);
        pending.push_str(&text);
        if let Some(line_end) = pending.rfind('\n') {
            pending.drain(..=line_end);
        }
        if !pending.trim_end().ends_with(':') {
            continue;
        }
        pending.clear();
        let Some(answer) = answers.get(answered) else {
            let _ = child.kill();
            break;
        };
        // A write that fails means the program has gone; its exit status is
        // what says why.
        if writeln!(input, "{answer}").is_err() {
            break;
        }
        answered += 1;
        said.clear();
    }
    drop(input);

    let status = child.wait().map_err(failed)?;
    if status.success() {
        return Ok(());
    }
    if answered <= 1 {
        return Err(PasswdError::WrongCurrent);
    }
    Err(PasswdError::Refused(refusal(&said)))
}

/// Why `passwd` refused, from what it printed after the last answer: a
/// quality module's reason ("BAD PASSWORD: …") over `passwd`'s generic summary
/// that follows it, and never the prompt it was left asking.
fn refusal(said: &str) -> String {
    let lines: Vec<&str> = said
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.ends_with(':'))
        .collect();
    lines
        .iter()
        .find(|line| line.starts_with("BAD PASSWORD"))
        .or_else(|| lines.first())
        .map(|line| line.to_string())
        .unwrap_or_else(|| otto_kit::t!("settings-account-password-failed").to_string())
}

/// Re-read who this is: `/etc/passwd` first, which needs nothing running, then
/// AccountsService where it answers.
fn refresh() {
    let mut fresh = local_profile();
    let looked_up = lookup_accounts(&mut fresh);
    {
        let mut state = state().lock().unwrap();
        state.user = fresh.user;
        state.real_name = fresh.real_name;
        state.picture = fresh.picture;
        state.administrator = fresh.administrator;
        state.object = fresh.object;
        state.looked_up = true;
        if let Err(why) = looked_up {
            eprintln!("account: AccountsService is not answering ({why})");
        }
    }
    changed();
}

fn lookup_accounts(profile: &mut State) -> Result<(), String> {
    let connection = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
    let reply = connection
        .call_method(
            Some(BUS_NAME),
            "/org/freedesktop/Accounts",
            Some(BUS_NAME),
            "FindUserById",
            &(uid() as i64,),
        )
        .map_err(|e| e.to_string())?;
    let object: zbus::zvariant::OwnedObjectPath =
        reply.body().deserialize().map_err(|e| e.to_string())?;
    let user = user_proxy(&connection, object.as_str())?;
    let get = |name: &str| -> Result<zbus::zvariant::OwnedValue, String> {
        user.get_property(name).map_err(|e| e.to_string())
    };
    if let Ok(name) = String::try_from(get("RealName")?) {
        if !name.is_empty() {
            profile.real_name = name;
        }
    }
    if let Ok(icon) = String::try_from(get("IconFile")?) {
        // The service reports where an icon *would* be even when there is
        // none, so it is only taken when there is a file behind it.
        profile.picture = if Path::new(&icon).is_file() {
            icon
        } else {
            String::new()
        };
    }
    if let Ok(kind) = i32::try_from(get("AccountType")?) {
        profile.administrator = kind == 1;
    }
    profile.object = Some(object.as_str().to_string());
    Ok(())
}

/// A proxy that reads properties fresh rather than from a cache, since every
/// read here follows a change.
fn user_proxy<'a>(
    connection: &'a zbus::blocking::Connection,
    object: &'a str,
) -> Result<zbus::blocking::Proxy<'a>, String> {
    zbus::blocking::proxy::Builder::new(connection)
        .destination(BUS_NAME)
        .and_then(|b| b.path(object))
        .and_then(|b| b.interface(USER_INTERFACE))
        .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No))
        .and_then(|b| b.build())
        .map_err(|e| e.to_string())
}

fn call<B>(object: &str, method: &str, body: &B) -> Result<(), String>
where
    B: serde::ser::Serialize + zbus::zvariant::DynamicType,
{
    let connection = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
    connection
        .call_method(Some(BUS_NAME), object, Some(USER_INTERFACE), method, body)
        .map(|_| ())
        .map_err(|err| match err {
            // polkit's refusal names the action, which says nothing to anyone
            // but an administrator; the error name is enough to know it.
            zbus::Error::MethodError(name, _, _) if name.contains("PermissionDenied") => {
                otto_kit::t!("settings-account-not-permitted").to_string()
            }
            other => other.to_string(),
        })
}

/// Who this is according to `/etc/passwd`, and their picture according to the
/// places a desktop keeps one — what is shown before the bus answers, and all
/// there is without it.
fn local_profile() -> State {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    let entry = std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|passwd| {
            passwd
                .lines()
                .map(|line| line.split(':').map(str::to_string).collect::<Vec<_>>())
                .find(|fields| fields.first() == Some(&user))
        });
    let real_name = entry
        .as_ref()
        .and_then(|fields| fields.get(4))
        .and_then(|gecos| gecos.split(',').next())
        .map(|name| name.trim().to_string())
        .unwrap_or_default();
    let picture = [
        PathBuf::from("/var/lib/AccountsService/icons").join(&user),
        face_path().unwrap_or_default(),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .map(|path| path.to_string_lossy().into_owned())
    .unwrap_or_default();

    State {
        administrator: in_admin_group(&user),
        user,
        real_name,
        picture,
        ..State::default()
    }
}

/// Whether `user` is in the group that may administer the machine — `wheel`
/// on Arch and Fedora, `sudo` on Debian — which is what AccountsService itself
/// goes by.
fn in_admin_group(user: &str) -> bool {
    std::fs::read_to_string("/etc/group").is_ok_and(|groups| {
        groups.lines().any(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            matches!(fields.first(), Some(&"wheel" | &"sudo" | &"admin"))
                && fields
                    .get(3)
                    .is_some_and(|members| members.split(',').any(|m| m == user))
        })
    })
}

fn uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .map(|m| m.uid())
        .unwrap_or(u32::MAX)
}

fn face_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".face"))
}

/// Cut `source` to a centred square, scale it to [`PICTURE_PX`] and write it
/// to `~/.face`, which is then what AccountsService is handed. The copy in
/// the home directory keeps the desktops and display managers that read
/// `~/.face` rather than the service in step with it.
fn write_face(source: &Path) -> Result<PathBuf, String> {
    let unreadable = || otto_kit::t!("settings-account-picture-unreadable").to_string();
    let bytes = std::fs::read(source).map_err(|_| unreadable())?;
    let image =
        skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&bytes)).ok_or_else(unreadable)?;

    let side = image.width().min(image.height());
    let crop = skia_safe::Rect::from_xywh(
        ((image.width() - side) / 2) as f32,
        ((image.height() - side) / 2) as f32,
        side as f32,
        side as f32,
    );
    let mut surface =
        skia_safe::surfaces::raster_n32_premul((PICTURE_PX, PICTURE_PX)).ok_or_else(unreadable)?;
    let mut paint = skia_safe::Paint::default();
    paint.set_anti_alias(true);
    surface.canvas().draw_image_rect_with_sampling_options(
        &image,
        Some((&crop, skia_safe::canvas::SrcRectConstraint::Fast)),
        skia_safe::Rect::from_wh(PICTURE_PX as f32, PICTURE_PX as f32),
        skia_safe::SamplingOptions::new(
            skia_safe::FilterMode::Linear,
            skia_safe::MipmapMode::Linear,
        ),
        &paint,
    );
    let png = surface
        .image_snapshot()
        .encode(None, skia_safe::EncodedImageFormat::PNG, 100)
        .ok_or_else(unreadable)?;

    let face = face_path().ok_or_else(unreadable)?;
    // Written beside it and renamed over it, so a reader never finds half an
    // image.
    let partial = face.with_extension("otto-partial");
    std::fs::write(&partial, png.as_bytes())
        .and_then(|()| std::fs::rename(&partial, &face))
        .map_err(|err| err.to_string())?;
    Ok(face)
}

fn remove_face() {
    if let Some(face) = face_path() {
        let _ = std::fs::remove_file(face);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for `passwd` that asks what the real one asks and checks the
    /// answers, so the conversation is exercised without touching an account.
    fn fake_passwd(dir: &Path, script: &str) -> String {
        let path = dir.join("passwd");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("otto-account-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const ASKS: &str = r#"
echo "Changing password for someone."
printf "Current password: "; read cur
[ "$cur" = "old" ] || { echo "passwd: Authentication token manipulation error"; exit 10; }
printf "New password: "; read a
printf "Retype new password: "; read b
[ "$a" = "$b" ] || { echo "Sorry, passwords do not match."; exit 10; }
[ ${#a} -ge 8 ] || { echo "BAD PASSWORD: The password is shorter than 8 characters"; printf "New password: "; read c; echo "passwd: Have exhausted maximum number of retries for service"; exit 10; }
echo "passwd: password updated successfully"
"#;

    #[test]
    fn the_prompts_are_answered_in_order() {
        let program = fake_passwd(&scratch("ok"), ASKS);
        assert_eq!(change_password(&program, "old", "a-long-new-one"), Ok(()));
    }

    #[test]
    fn stopping_after_the_current_password_means_it_was_wrong() {
        let program = fake_passwd(&scratch("wrong"), ASKS);
        assert_eq!(
            change_password(&program, "nope", "a-long-new-one"),
            Err(PasswdError::WrongCurrent)
        );
    }

    #[test]
    fn a_quality_refusal_is_reported_in_its_own_words_and_not_answered_again() {
        let program = fake_passwd(&scratch("weak"), ASKS);
        assert_eq!(
            change_password(&program, "old", "short"),
            Err(PasswdError::Refused(
                "BAD PASSWORD: The password is shorter than 8 characters".into()
            ))
        );
    }

    #[test]
    fn a_missing_program_is_a_refusal_not_a_hang() {
        assert!(matches!(
            change_password("/nonexistent/passwd", "a", "b"),
            Err(PasswdError::Refused(_))
        ));
    }

    #[test]
    fn only_the_password_fields_are_masked() {
        assert!(is_secret(CURRENT_ID) && is_secret(NEW_ID) && is_secret(CONFIRM_ID));
        assert!(!is_secret(NAME_ID) && !is_secret(PICTURE_ID));
        assert!(owns(NAME_ID) && !owns("dock.size"));
    }
}
