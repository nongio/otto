//! The users pane: everyone who can log in, in a list beside the one selected
//! — their picture, their name, their account type and their password.
//!
//! None of it is an Otto setting. Accounts belong to the system's user
//! database, and are read and written through AccountsService
//! (`org.freedesktop.Accounts`) because that is where the greeter and the lock
//! screen read them back from (`otto-auth-ui`'s `user.rs`: the AccountsService
//! icon, then `~/.face`; the real name from `/etc/passwd`, which
//! AccountsService keeps). Changing your own name and picture needs no
//! password under the stock polkit rules; anything done to another account —
//! adding it, deleting it, renaming it, changing its type or its password — is
//! user administration, which polkit asks an administrator to approve through
//! the session's agent (`otto-authorize`).
//!
//! Your own password is changed by `passwd`, the way every desktop that does
//! not ship its own PAM helper does it: the program runs on a thread of its own
//! and is answered prompt by prompt over a pipe. Another account's password is
//! set through AccountsService, which takes it already hashed. Either way the
//! passwords go through a pipe, never a command line, and are dropped from this
//! process as soon as the attempt is over, whichever way it went.
//!
//! The rows carry identifiers in an `account.` namespace the compositor does
//! not serve, so `main.rs` hands their edits here (see [`owns`]).

// Rust guideline compliant 2026-02-21

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_kit::components::selection_list::SelectionListHit;
use zeroize::Zeroizing;

use crate::model::{group, untitled, Control, Pane, Row};

pub const PICTURE_ID: &str = "account.picture";
const NAME_ID: &str = "account.real_name";
const TYPE_ID: &str = "account.type";
const CURRENT_ID: &str = "account.password.current";
const NEW_ID: &str = "account.password.new";
const CONFIRM_ID: &str = "account.password.confirm";
const ADD_NAME_ID: &str = "account.add.full_name";
const ADD_USER_ID: &str = "account.add.user_name";

const BUS_NAME: &str = "org.freedesktop.Accounts";
const MANAGER_PATH: &str = "/org/freedesktop/Accounts";
const USER_INTERFACE: &str = "org.freedesktop.Accounts.User";

/// AccountsService's `AccountType` for an administrator; 0 is a standard user.
const ADMINISTRATOR: i32 = 1;

/// The longest login name `useradd` accepts.
const USER_NAME_MAX: usize = 32;

/// The side of the square a chosen picture is cut to. Large enough for the
/// greeter's 96pt avatar at 2x with room to spare, small enough that
/// AccountsService — which refuses icons over a megabyte — takes it.
const PICTURE_PX: i32 = 256;

/// One account that can log in.
#[derive(Default, Clone, Debug, PartialEq)]
struct Account {
    /// The AccountsService object for it. `None` where the service is not
    /// running — the account is then shown but cannot be changed from here.
    object: Option<String>,
    uid: u32,
    /// Login name.
    user: String,
    real_name: String,
    /// The picture's path, empty when there is none.
    picture: String,
    administrator: bool,
}

impl Account {
    /// What the account is called where people read it: its full name, or its
    /// login name when it has none.
    fn display_name(&self) -> &str {
        if self.real_name.trim().is_empty() {
            &self.user
        } else {
            &self.real_name
        }
    }
}

/// Which sheet is up over the pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sheet {
    /// Your own password, through `passwd`.
    ChangePassword,
    /// Another account's password, set by an administrator.
    ResetPassword,
    AddUser,
    DeleteUser,
}

impl Sheet {
    /// The sheet's fields, in the order Tab and Enter walk them.
    fn fields(self) -> &'static [&'static str] {
        match self {
            Sheet::ChangePassword => &[CURRENT_ID, NEW_ID, CONFIRM_ID],
            Sheet::ResetPassword => &[NEW_ID, CONFIRM_ID],
            Sheet::AddUser => &[ADD_NAME_ID, ADD_USER_ID, NEW_ID, CONFIRM_ID],
            Sheet::DeleteUser => &[],
        }
    }
}

/// How the work a sheet started is going.
#[derive(Default, Clone, PartialEq, Debug)]
enum Status {
    #[default]
    Idle,
    Working,
    Failed(String),
}

/// What the pane shows and what has been typed into its sheet so far.
#[derive(Default)]
struct State {
    /// Everyone who can log in; you are always first.
    accounts: Vec<Account>,
    /// Index into `accounts` of the one the detail shows.
    selected: usize,
    /// A login name to select once the next refresh lists it: an account just
    /// added.
    select_after_refresh: Option<String>,
    /// Whether the lookup has finished, so "not running" is not claimed of a
    /// service that simply has not answered yet.
    looked_up: bool,
    /// Why the last change to a name, picture or type did not take.
    profile_error: Option<String>,
    sheet: Option<Sheet>,
    current: String,
    new: String,
    confirm: String,
    add_name: String,
    add_user: String,
    status: Status,
    /// Your password was changed this session, which the Password row says.
    password_changed: bool,
    /// Set by Cancel to stop the `passwd` underway, while there is one.
    cancel: Option<Arc<AtomicBool>>,
}

impl State {
    fn me(&self) -> &Account {
        &self.accounts[0]
    }

    fn shown(&self) -> &Account {
        self.accounts.get(self.selected).unwrap_or(self.me())
    }

    /// Whether you may administer other accounts: you are an administrator
    /// and AccountsService is there to do it.
    fn can_administer(&self) -> bool {
        self.me().administrator && self.me().object.is_some()
    }

    fn clear_fields(&mut self) {
        for field in [
            &mut self.current,
            &mut self.new,
            &mut self.confirm,
            &mut self.add_name,
            &mut self.add_user,
        ] {
            field.clear();
        }
    }

    fn field_mut(&mut self, id: &str) -> Option<&mut String> {
        Some(match id {
            CURRENT_ID => &mut self.current,
            NEW_ID => &mut self.new,
            CONFIRM_ID => &mut self.confirm,
            ADD_NAME_ID => &mut self.add_name,
            ADD_USER_ID => &mut self.add_user,
            _ => return None,
        })
    }
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        let state = Mutex::new(State {
            accounts: vec![local_profile()],
            ..State::default()
        });
        // The first read off the bus happens away from the main thread; the
        // pane draws what `/etc/passwd` says until it answers.
        spawn("account-lookup", refresh);
        state
    })
}

static DIRTY: AtomicBool = AtomicBool::new(false);

/// Set when a sheet with fields opens, so `main.rs` can put the keyboard in
/// its first one — the toolkit's editor lives there, not here.
static SHEET_OPENED: AtomicBool = AtomicBool::new(false);

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

fn type_label(administrator: bool) -> &'static str {
    if administrator {
        otto_kit::t!("settings-account-type-administrator")
    } else {
        otto_kit::t!("settings-account-type-standard")
    }
}

pub fn build() -> Pane {
    let state = state().lock().unwrap();
    let shown = state.shown();
    let mine = state.selected == 0;
    let editable = shown.object.is_some() && (mine || state.can_administer());

    let name = Row::new(
        otto_kit::t!("settings-account-full-name"),
        if editable {
            Control::Text(shown.real_name.clone())
        } else {
            Control::Value(shown.real_name.clone())
        },
    )
    .id(NAME_ID);
    let name = match (
        &state.profile_error,
        shown.object.is_some(),
        state.looked_up,
    ) {
        (Some(error), _, _) => name.detail(error.clone()),
        (None, false, true) => name.detail(otto_kit::t!("settings-account-no-accountsservice")),
        _ => name,
    };

    // Nobody changes their own type from here: an administrator demoting
    // themselves could leave the machine with none.
    let kind = Row::new(
        otto_kit::t!("settings-account-type"),
        if !mine && state.can_administer() {
            Control::Select(type_label(shown.administrator).to_string())
        } else {
            Control::Value(type_label(shown.administrator).to_string())
        },
    )
    .id(TYPE_ID);

    let mut groups = vec![untitled(vec![
        name,
        Row::new(
            otto_kit::t!("settings-account-name"),
            Control::Value(shown.user.clone()),
        ),
        kind,
    ])];

    if mine {
        // The row reports how the last change went; the sheet reports a
        // change still being made, or refused, while it is up.
        let status = if state.password_changed {
            otto_kit::t!("settings-account-password-changed")
        } else {
            otto_kit::t!("settings-account-password-detail")
        };
        groups.push(group(
            otto_kit::t!("settings-group-password"),
            vec![Row::new(password_label(), Control::Button(change_buttons())).detail(status)],
        ));
    } else if state.can_administer() {
        groups.push(group(
            otto_kit::t!("settings-group-password"),
            vec![Row::new(password_label(), Control::Button(reset_buttons()))
                .detail(otto_kit::t!("settings-account-reset-detail"))],
        ));
    }

    Pane {
        name: otto_kit::t!("settings-pane-account"),
        icon: "person",
        intro: None,
        groups,
    }
}

/// The Password row's label.
fn password_label() -> &'static str {
    otto_kit::t!("settings-group-password")
}

fn change_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![otto_kit::t!("settings-account-change-password-ellipsis")])
}

fn reset_buttons() -> &'static [&'static str] {
    static BUTTONS: OnceLock<Vec<&'static str>> = OnceLock::new();
    BUTTONS.get_or_init(|| vec![otto_kit::t!("settings-account-reset-password-ellipsis")])
}

/// One account in the users list.
#[derive(Debug, Clone, PartialEq)]
pub struct UserItem {
    pub name: String,
    /// Its type, and whether it is you.
    pub subtitle: String,
    /// The picture's path, empty when there is none.
    pub picture: String,
}

/// The users list beside the detail.
#[derive(Debug, Clone, PartialEq)]
pub struct UsersView {
    pub items: Vec<UserItem>,
    pub selected: usize,
    pub can_add: bool,
    /// You are an administrator and someone other than you is selected.
    pub can_remove: bool,
}

/// What the users list shows.
pub fn users() -> UsersView {
    let state = state().lock().unwrap();
    let items = state
        .accounts
        .iter()
        .enumerate()
        .map(|(i, account)| UserItem {
            name: account.display_name().to_string(),
            subtitle: if i == 0 {
                otto_kit::t_owned!(
                    "settings-users-you",
                    kind = type_label(account.administrator)
                )
            } else {
                type_label(account.administrator).to_string()
            },
            picture: account.picture.clone(),
        })
        .collect();
    UsersView {
        items,
        selected: state.selected,
        can_add: state.can_administer(),
        can_remove: state.can_administer() && state.selected != 0,
    }
}

/// The header over the detail: the selected account's picture and name.
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderView {
    pub name: String,
    pub picture: String,
    /// Whether its Choose… button does anything.
    pub can_choose: bool,
}

/// What the header over the detail shows.
pub fn header() -> HeaderView {
    let state = state().lock().unwrap();
    let shown = state.shown();
    HeaderView {
        name: shown.display_name().to_string(),
        picture: shown.picture.clone(),
        can_choose: shown.object.is_some() && (state.selected == 0 || state.can_administer()),
    }
}

/// A press on the users list. Selecting an account happens at once, as the
/// sidebar's does; the add and remove buttons wait for [`activate`].
pub fn press_list(hit: SelectionListHit) {
    let SelectionListHit::Item(index) = hit else {
        return;
    };
    let mut state = state().lock().unwrap();
    if index < state.accounts.len() && index != state.selected {
        state.selected = index;
        state.profile_error = None;
        drop(state);
        changed();
    }
}

/// The add or remove button was pressed and released: open its sheet.
pub fn activate(hit: SelectionListHit) {
    let held = state().lock().unwrap();
    let sheet = match hit {
        SelectionListHit::Add if held.can_administer() => Sheet::AddUser,
        SelectionListHit::Remove if held.can_administer() && held.selected != 0 => {
            Sheet::DeleteUser
        }
        _ => return,
    };
    drop(held);
    open_sheet(sheet);
}

fn open_sheet(sheet: Sheet) {
    let mut state = state().lock().unwrap();
    state.sheet = Some(sheet);
    state.clear_fields();
    state.status = Status::Idle;
    drop(state);
    if !sheet.fields().is_empty() {
        SHEET_OPENED.store(true, Ordering::Relaxed);
    }
    changed();
}

/// Whether `id` is one of this pane's rows rather than a compositor setting.
pub fn owns(id: &str) -> bool {
    id.starts_with("account.")
}

/// Whether a field's contents must be masked: the password fields.
pub fn is_secret(id: &str) -> bool {
    matches!(id, CURRENT_ID | NEW_ID | CONFIRM_ID)
}

/// A field on this pane was committed.
pub fn commit_text(id: &str, text: &str) {
    let mut state = state().lock().unwrap();
    if id == NAME_ID {
        let name = text.trim().to_string();
        let selected = state.selected;
        let account = &mut state.accounts[selected];
        if name == account.real_name {
            return;
        }
        let Some(object) = account.object.clone() else {
            return;
        };
        // Shown at once; the refresh after the call puts back whatever the
        // service actually kept.
        account.real_name = name.clone();
        drop(state);
        changed();
        spawn("account-name", move || {
            let outcome = call(&object, "SetRealName", &(name.as_str(),));
            settle(outcome);
        });
        return;
    }
    let suggest = id == ADD_NAME_ID && state.add_user.is_empty();
    let Some(field) = state.field_mut(id) else {
        return;
    };
    *field = text.to_string();
    // A full name typed into an empty form suggests its login name.
    if suggest {
        state.add_user = suggested_user_name(text);
    }
    // Typing again after an attempt starts a new one.
    if !matches!(state.status, Status::Working) {
        state.status = Status::Idle;
    }
}

/// A login name made from a full name: its first word, lower case, with
/// anything `useradd` would refuse left out.
fn suggested_user_name(full_name: &str) -> String {
    let first = full_name.split_whitespace().next().unwrap_or_default();
    let name: String = first
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
        .take(USER_NAME_MAX)
        .collect();
    name.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-')
        .to_string()
}

/// Whether `name` is a login name `useradd` takes as it stands: it starts with
/// a lower-case letter or an underscore, and goes on in lower-case letters,
/// digits, underscores and hyphens.
fn valid_user_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && name.len() <= USER_NAME_MAX
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
}

/// The picture was chosen, or removed (an empty path). Returns whether `id`
/// was this pane's, so `main::apply` can stop there.
pub fn apply(id: &str, value: &crate::settings_client::Value) -> bool {
    if id != PICTURE_ID {
        return owns(id);
    }
    let crate::settings_client::Value::Text(path) = value else {
        return true;
    };
    let held = state().lock().unwrap();
    let mine = held.selected == 0;
    let shown = held.shown().clone();
    drop(held);
    let Some(object) = shown.object else {
        return true;
    };
    let path = path.clone();
    spawn("account-picture", move || {
        let outcome = if path.is_empty() {
            if mine {
                remove_face();
            }
            call(&object, "SetIconFile", &("",))
        } else {
            // Your own picture is kept as `~/.face` too; another account's is
            // cut to a scratch file the service copies in and is then
            // removed.
            let target = if mine {
                face_path()
            } else {
                scratch_face(&shown.user)
            };
            let outcome = match target {
                Some(target) => write_face(Path::new(&path), &target).and_then(|face| {
                    call(&object, "SetIconFile", &(face.to_string_lossy().as_ref(),))
                }),
                None => Err(otto_kit::t!("settings-account-picture-unreadable").to_string()),
            };
            if !mine {
                if let Some(scratch) = scratch_face(&shown.user) {
                    let _ = std::fs::remove_file(scratch);
                }
            }
            outcome
        };
        settle(outcome);
    });
    true
}

/// The choices for this pane's pop-up: the account types.
pub fn menu_choices(id: &str) -> Option<Vec<crate::discovery::Choice>> {
    (id == TYPE_ID).then(|| {
        [false, true]
            .into_iter()
            .map(|administrator| crate::discovery::Choice {
                label: type_label(administrator).to_string(),
                value: type_label(administrator).to_string(),
            })
            .collect()
    })
}

/// A type was picked for the selected account. Whether `id` was this pane's.
pub fn choose(id: &str, value: &str) -> bool {
    if id != TYPE_ID {
        return false;
    }
    let mut state = state().lock().unwrap();
    let administrator = value == type_label(true);
    let selected = state.selected;
    if selected == 0 || !state.can_administer() {
        return true;
    }
    let account = &mut state.accounts[selected];
    if account.administrator == administrator {
        return true;
    }
    let Some(object) = account.object.clone() else {
        return true;
    };
    account.administrator = administrator;
    drop(state);
    changed();
    let kind = if administrator { ADMINISTRATOR } else { 0 };
    spawn("account-type", move || {
        settle(call(&object, "SetAccountType", &(kind,)));
    });
    true
}

/// Record how a change to an account went, and re-read them all.
fn settle(outcome: Result<(), String>) {
    state().lock().unwrap().profile_error = outcome.err();
    refresh();
}

/// A press on this pane's push buttons: Change Password… and Reset Password…
/// open their sheets.
pub fn press(row: &str, button: &str) {
    if row != password_label() {
        return;
    }
    if change_buttons().contains(&button) {
        state().lock().unwrap().password_changed = false;
        open_sheet(Sheet::ChangePassword);
    } else if reset_buttons().contains(&button) {
        open_sheet(Sheet::ResetPassword);
    }
}

/// Whether a sheet with fields has just opened, once: `main.rs` then puts the
/// keyboard in its first field.
pub fn take_sheet_opened() -> bool {
    SHEET_OPENED.swap(false, Ordering::Relaxed)
}

/// The fields of the sheet that is up, in the order Tab and Enter walk them;
/// empty when none is.
pub fn sheet_fields() -> &'static [&'static str] {
    state().lock().unwrap().sheet.map_or(&[], Sheet::fields)
}

/// One field of a sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetField {
    pub id: &'static str,
    pub label: &'static str,
    /// What the field shows at rest: its text, or a dot per character for a
    /// password.
    pub shown: String,
}

/// What the sheet that is up shows.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetView {
    pub title: String,
    /// A paragraph under the title, for a sheet that asks rather than takes
    /// input.
    pub body: Option<String>,
    pub fields: Vec<SheetField>,
    /// The default button's label.
    pub action: &'static str,
    /// The default button destroys something, so it is drawn in red.
    pub destructive: bool,
    /// A line under the fields: the work underway, or why it was refused.
    pub message: Option<(String, bool)>,
    /// Work is underway, so the default button does nothing.
    pub busy: bool,
    /// Cancel still works while busy: it stops the work underway.
    pub cancellable: bool,
}

/// The sheet, while one is up.
pub fn sheet() -> Option<SheetView> {
    let state = state().lock().unwrap();
    let sheet = state.sheet?;
    let shown = state.shown().display_name().to_string();
    let field = |id: &'static str| {
        let (label, text) = match id {
            CURRENT_ID => (
                otto_kit::t!("settings-account-current-password"),
                &state.current,
            ),
            NEW_ID => (otto_kit::t!("settings-account-new-password"), &state.new),
            CONFIRM_ID => (
                otto_kit::t!("settings-account-confirm-password"),
                &state.confirm,
            ),
            ADD_NAME_ID => (otto_kit::t!("settings-account-full-name"), &state.add_name),
            _ => (otto_kit::t!("settings-account-name"), &state.add_user),
        };
        SheetField {
            id,
            label,
            shown: if is_secret(id) {
                "\u{2022}".repeat(text.chars().count())
            } else {
                text.clone()
            },
        }
    };
    let (title, body, action, destructive) = match sheet {
        Sheet::ChangePassword => (
            otto_kit::t_owned!("settings-account-change-password"),
            None,
            otto_kit::t!("settings-account-change-password"),
            false,
        ),
        Sheet::ResetPassword => (
            otto_kit::t_owned!("settings-users-reset-title", name = shown.clone()),
            None,
            otto_kit::t!("settings-users-reset-action"),
            false,
        ),
        Sheet::AddUser => (
            otto_kit::t_owned!("settings-users-add-title"),
            None,
            otto_kit::t!("settings-users-add-action"),
            false,
        ),
        Sheet::DeleteUser => (
            otto_kit::t_owned!("settings-users-delete-title", name = shown.clone()),
            Some(otto_kit::t_owned!("settings-users-delete-body")),
            otto_kit::t!("settings-users-delete-action"),
            true,
        ),
    };
    Some(SheetView {
        title,
        body,
        fields: sheet.fields().iter().map(|id| field(id)).collect(),
        action,
        destructive,
        message: match &state.status {
            Status::Working if sheet == Sheet::ChangePassword => Some((
                otto_kit::t_owned!("settings-account-password-changing"),
                false,
            )),
            // Anything done to another account waits on an administrator's
            // approval, which can take as long as they take.
            Status::Working => Some((otto_kit::t_owned!("settings-account-working"), false)),
            Status::Failed(why) => Some((why.clone(), true)),
            Status::Idle => None,
        },
        busy: state.status == Status::Working,
        cancellable: state.status != Status::Working || state.cancel.is_some(),
    })
}

/// What a sheet field holds, for the editor to start from.
pub fn field_value(id: &str) -> String {
    state()
        .lock()
        .unwrap()
        .field_mut(id)
        .map(|field| field.clone())
        .unwrap_or_default()
}

/// Close the sheet without doing anything — Cancel, or Escape. While a
/// password is being changed it stops `passwd`; other work is waited out, as
/// closing would hide how it ends.
pub fn close_sheet() {
    let mut state = state().lock().unwrap();
    if state.status == Status::Working {
        let Some(cancel) = state.cancel.take() else {
            return;
        };
        cancel.store(true, Ordering::Relaxed);
    }
    state.sheet = None;
    state.clear_fields();
    state.status = Status::Idle;
    drop(state);
    changed();
}

/// The sheet's default button: check what was typed, then do it. The sheet
/// closes when it is done, and stays up saying why when it is not.
pub fn submit() {
    let mut held = state().lock().unwrap();
    let Some(sheet) = held.sheet else {
        return;
    };
    if held.status == Status::Working {
        return;
    }
    if let Some(problem) = problem(&held, sheet) {
        held.status = Status::Failed(problem.to_string());
        drop(held);
        changed();
        return;
    }

    // Taken out of the sheet as the work starts: they live on only in the
    // thread doing it, and go when it does.
    let current = std::mem::take(&mut held.current);
    let new = std::mem::take(&mut held.new);
    let add_name = std::mem::take(&mut held.add_name);
    let add_user = std::mem::take(&mut held.add_user);
    held.confirm.clear();
    held.status = Status::Working;
    let cancel = Arc::new(AtomicBool::new(false));
    if sheet == Sheet::ChangePassword {
        held.cancel = Some(cancel.clone());
    }
    let shown = held.shown().clone();
    drop(held);
    changed();

    spawn("account-sheet", move || {
        let outcome = match sheet {
            Sheet::ChangePassword => {
                change_password("passwd", &current, &new, &cancel, PASSWD_PATIENCE).map_err(|err| {
                    match err {
                        PasswdError::WrongCurrent => {
                            otto_kit::t!("settings-account-password-wrong-current").to_string()
                        }
                        PasswdError::Refused(why) => why,
                    }
                })
            }
            Sheet::ResetPassword => match &shown.object {
                Some(object) => set_password(object, &new),
                None => Err(otto_kit::t!("settings-account-no-accountsservice").to_string()),
            },
            Sheet::AddUser => add_user_account(&add_user, &add_name, &new),
            Sheet::DeleteUser => delete_user_account(shown.uid),
        };
        let mut state = state().lock().unwrap();
        // Cancelled: the sheet is already closed, and may be up again for
        // another try that this must not touch.
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        state.cancel = None;
        match outcome {
            Ok(()) => {
                state.sheet = None;
                state.status = Status::Idle;
                match sheet {
                    Sheet::ChangePassword => state.password_changed = true,
                    Sheet::AddUser => state.select_after_refresh = Some(add_user),
                    Sheet::DeleteUser => state.selected = 0,
                    Sheet::ResetPassword => {}
                }
            }
            Err(why) => {
                // Asked again from the top: the new user's names are put
                // back, the passwords are not.
                if sheet == Sheet::AddUser {
                    state.add_name = add_name;
                    state.add_user = add_user;
                }
                state.status = Status::Failed(why);
            }
        }
        drop(state);
        changed();
        if sheet != Sheet::ChangePassword {
            refresh();
        }
    });
}

/// What is wrong with the sheet's fields as they stand, if anything.
fn problem(state: &State, sheet: Sheet) -> Option<&'static str> {
    let passwords = || {
        if state.new.is_empty() {
            Some(otto_kit::t!("settings-users-password-missing"))
        } else if state.new != state.confirm {
            Some(otto_kit::t!("settings-account-password-mismatch"))
        } else {
            None
        }
    };
    match sheet {
        Sheet::ChangePassword => {
            if state.current.is_empty() || state.new.is_empty() {
                Some(otto_kit::t!("settings-account-password-missing"))
            } else if state.new == state.current && !state.new.is_empty() {
                Some(otto_kit::t!("settings-account-password-same"))
            } else {
                passwords()
            }
        }
        Sheet::ResetPassword => passwords(),
        Sheet::AddUser => {
            if !valid_user_name(&state.add_user) {
                Some(otto_kit::t!("settings-users-invalid-name"))
            } else if state.accounts.iter().any(|a| a.user == state.add_user) {
                Some(otto_kit::t!("settings-users-name-taken"))
            } else {
                passwords()
            }
        }
        Sheet::DeleteUser => None,
    }
}

/// Hash `password` the way `/etc/shadow` keeps it (SHA-512 crypt), which is
/// what AccountsService's `SetPassword` takes.
///
/// The salt is 12 random bytes, 16 characters once encoded: glibc's `crypt`
/// reads no more than that, and `pam_unix` compares its output with the
/// stored hash as a string, so a longer salt would never match.
fn hash_password(password: &str) -> Result<Zeroizing<String>, String> {
    use sha_crypt::{password_hash, PasswordHasher, ShaCrypt};
    let failed = || otto_kit::t!("settings-account-password-failed").to_string();
    let salt = password_hash::try_generate_salt().map_err(|_| failed())?;
    ShaCrypt::SHA512
        .hash_password_with_salt(password.as_bytes(), &salt[..12])
        .map(|hash| Zeroizing::new(hash.to_string()))
        .map_err(|_| failed())
}

fn set_password(object: &str, password: &str) -> Result<(), String> {
    let hash = hash_password(password)?;
    call(object, "SetPassword", &(hash.as_str(), ""))
}

/// Make a standard account, then give it its password.
///
/// The password is hashed first, so nothing is created when it cannot be;
/// an account left without one is deleted again, so a retry does not find
/// the name taken.
fn add_user_account(user: &str, full_name: &str, password: &str) -> Result<(), String> {
    let hash = hash_password(password)?;
    let full_name = if full_name.trim().is_empty() {
        user
    } else {
        full_name.trim()
    };
    let object: zbus::zvariant::OwnedObjectPath = administer(
        MANAGER_PATH,
        BUS_NAME,
        "CreateUser",
        &(user, full_name, 0_i32),
    )?
    .ok_or_else(|| otto_kit::t!("settings-account-not-permitted").to_string())?;
    call(object.as_str(), "SetPassword", &(hash.as_str(), "")).inspect_err(|_| {
        let uid = zbus::blocking::Connection::system()
            .ok()
            .and_then(|connection| read_account(&connection, object.as_str()).ok())
            .map(|account| account.uid)
            .filter(|&uid| uid != u32::MAX);
        if let Some(uid) = uid {
            let _ = delete_user_account(uid);
        }
    })
}

/// Delete an account, keeping its home folder.
fn delete_user_account(uid: u32) -> Result<(), String> {
    administer::<_, ()>(
        MANAGER_PATH,
        BUS_NAME,
        "DeleteUser",
        &(i64::from(uid), false),
    )
    .map(|_| ())
}

/// How long `passwd` may go without saying anything before it is given up on.
/// Long enough for PAM's delay after a wrong password and for hashing the new
/// one, short enough that a prompt not recognised as one does not hang the
/// sheet.
const PASSWD_PATIENCE: Duration = Duration::from_secs(20);

/// How often a wait on `passwd` looks up to see whether it was cancelled.
const CANCEL_POLL: Duration = Duration::from_millis(100);

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
/// line break, after a colon (ASCII or fullwidth). That holds in every
/// language PAM is translated into, so the user's locale is left alone and a
/// refusal comes back in it. A fourth prompt is a password-quality module
/// asking again after refusing the new one; it is not answered, since the
/// answer would be the same.
///
/// `program` is killed when `cancel` is set, or when it stays quiet for
/// `patience` — waiting on something not recognised as a prompt.
fn change_password(
    program: &str,
    current: &str,
    new: &str,
    cancel: &AtomicBool,
    patience: Duration,
) -> Result<(), PasswdError> {
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
        spawn_retrying_busy(&mut command).map_err(failed)?
    };
    let mut input = child.stdin.take().expect("stdin is piped");

    // Read on a thread of its own so the wait for output can be given up on;
    // the channel closes when the output ends.
    let (send, chunks) = mpsc::channel();
    spawn("passwd-output", move || {
        let mut buffer = [0u8; 512];
        while let Ok(read @ 1..) = output.read(&mut buffer) {
            if send.send(buffer[..read].to_vec()).is_err() {
                break;
            }
        }
    });

    let answers = [current, new, new];
    let mut answered = 0;
    // What it has said since the last answer, which is where a reason for
    // refusing ends up.
    let mut said = String::new();
    let mut pending = String::new();
    let mut deadline = Instant::now() + patience;
    loop {
        if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PasswdError::Refused(
                otto_kit::t!("settings-account-password-failed").to_string(),
            ));
        }
        let wait = deadline.saturating_duration_since(Instant::now());
        let chunk = match chunks.recv_timeout(wait.min(CANCEL_POLL)) {
            Ok(chunk) => chunk,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        deadline = Instant::now() + patience;
        let text = String::from_utf8_lossy(&chunk);
        said.push_str(&text);
        pending.push_str(&text);
        if let Some(line_end) = pending.rfind('\n') {
            pending.drain(..=line_end);
        }
        if !is_prompt(&pending) {
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
    // Stopping right after the current password means it was wrong; stopping
    // before it is not about the password, and is said in its own words.
    if answered == 1 {
        return Err(PasswdError::WrongCurrent);
    }
    Err(PasswdError::Refused(refusal(&said)))
}

/// Spawn `command`, trying again a few times while the kernel says the
/// program is busy (ETXTBSY): a program written moments ago can still be
/// held open for writing by another thread's fork that has not reached exec
/// yet. The tests' stand-in `passwd` hits this when tests run in parallel.
fn spawn_retrying_busy(command: &mut Command) -> std::io::Result<std::process::Child> {
    const TRIES: u32 = 10;
    let mut tried = 1;
    loop {
        match command.spawn() {
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy && tried < TRIES => {
                tried += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            outcome => return outcome,
        }
    }
}

/// Whether `text` — output since the last line break — is left asking for
/// something: it ends in a colon, ASCII or fullwidth.
fn is_prompt(text: &str) -> bool {
    text.trim_end().ends_with([':', '：'])
}

/// Why `passwd` refused, from what it printed after the last answer: a
/// quality module's reason ("BAD PASSWORD: …") over `passwd`'s generic summary
/// that follows it, and never the prompt it was left asking.
fn refusal(said: &str) -> String {
    let lines: Vec<&str> = said
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !is_prompt(line))
        .collect();
    lines
        .iter()
        .find(|line| line.starts_with("BAD PASSWORD"))
        .or_else(|| lines.first())
        .map(|line| line.to_string())
        .unwrap_or_else(|| otto_kit::t!("settings-account-password-failed").to_string())
}

/// Re-read who can log in: `/etc/passwd` first for you, which needs nothing
/// running, then every account AccountsService lists where it answers.
fn refresh() {
    let me = local_profile();
    let found = lookup_accounts(me.uid);
    let mut state = state().lock().unwrap();
    let previous = state.shown().uid;
    match found {
        Ok(accounts) => state.accounts = accounts,
        Err(why) => {
            eprintln!("account: AccountsService is not answering ({why})");
            state.accounts = vec![me];
        }
    }
    state.looked_up = true;
    // The selection follows the account, not its place in the list: an
    // account added or deleted moves the others.
    let wanted = state.select_after_refresh.take();
    let position = |state: &State| match &wanted {
        Some(user) => state.accounts.iter().position(|a| &a.user == user),
        None => state.accounts.iter().position(|a| a.uid == previous),
    };
    state.selected = position(&state).unwrap_or(0);
    drop(state);
    changed();
}

/// Everyone AccountsService lists, you first and then the others by name.
fn lookup_accounts(uid: u32) -> Result<Vec<Account>, String> {
    let connection = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
    let me: zbus::zvariant::OwnedObjectPath = connection
        .call_method(
            Some(BUS_NAME),
            MANAGER_PATH,
            Some(BUS_NAME),
            "FindUserById",
            &(i64::from(uid),),
        )
        .and_then(|reply| reply.body().deserialize())
        .map_err(|e| e.to_string())?;
    // Only the human accounts are cached; a failure here still leaves you.
    let others: Vec<zbus::zvariant::OwnedObjectPath> = connection
        .call_method(
            Some(BUS_NAME),
            MANAGER_PATH,
            Some(BUS_NAME),
            "ListCachedUsers",
            &(),
        )
        .and_then(|reply| reply.body().deserialize())
        .unwrap_or_default();

    let mut accounts = vec![read_account(&connection, me.as_str())?];
    let mut rest: Vec<Account> = others
        .iter()
        .filter(|object| object.as_str() != me.as_str())
        .filter_map(|object| read_account(&connection, object.as_str()).ok())
        .collect();
    rest.sort_by_key(|account| account.display_name().to_lowercase());
    accounts.extend(rest);
    Ok(accounts)
}

fn read_account(connection: &zbus::blocking::Connection, object: &str) -> Result<Account, String> {
    let user = user_proxy(connection, object)?;
    let get = |name: &str| -> Result<zbus::zvariant::OwnedValue, String> {
        user.get_property(name).map_err(|e| e.to_string())
    };
    let icon = String::try_from(get("IconFile")?).unwrap_or_default();
    Ok(Account {
        object: Some(object.to_string()),
        uid: u64::try_from(get("Uid")?)
            .ok()
            .and_then(|uid| u32::try_from(uid).ok())
            .unwrap_or(u32::MAX),
        user: String::try_from(get("UserName")?).unwrap_or_default(),
        real_name: String::try_from(get("RealName")?).unwrap_or_default(),
        // The service reports where an icon *would* be even when there is
        // none, so it is only taken when there is a file behind it.
        picture: if Path::new(&icon).is_file() {
            icon
        } else {
            String::new()
        },
        administrator: i32::try_from(get("AccountType")?).is_ok_and(|kind| kind == ADMINISTRATOR),
    })
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
    B: zbus::export::serde::ser::Serialize + zbus::zvariant::DynamicType,
{
    administer::<_, ()>(object, USER_INTERFACE, method, body).map(|_| ())
}

/// Call `method` on an AccountsService object, letting polkit ask for a
/// password.
///
/// The call carries D-Bus's allow-interactive-authorization flag. Without it
/// AccountsService tells polkit not to ask, and anything needing an
/// administrator — adding, deleting or changing another account — is refused
/// at once instead of prompting through the session's agent. The reply can
/// take as long as the person at the prompt does, which is why every caller
/// is on a thread of its own.
fn administer<B, R>(
    object: &str,
    interface: &str,
    method: &str,
    body: &B,
) -> Result<Option<R>, String>
where
    B: zbus::export::serde::ser::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
{
    let connection = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
    let proxy = zbus::blocking::proxy::Builder::<zbus::blocking::Proxy<'_>>::new(&connection)
        .destination(BUS_NAME)
        .and_then(|b| b.path(object))
        .and_then(|b| b.interface(interface))
        .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No))
        .and_then(|b| b.build())
        .map_err(|e| e.to_string())?;
    proxy
        .call_with_flags(
            method,
            zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
            body,
        )
        .map_err(denied)
}

/// A failed call in words for the pane.
fn denied(err: zbus::Error) -> String {
    match err {
        // polkit's refusal names the action, which says nothing to anyone but
        // an administrator; the error name is enough to know it.
        zbus::Error::MethodError(name, _, _) if name.contains("PermissionDenied") => {
            otto_kit::t!("settings-account-not-permitted").to_string()
        }
        zbus::Error::MethodError(_, Some(message), _) => message,
        other => other.to_string(),
    }
}

/// Who you are according to `/etc/passwd`, and your picture according to the
/// places a desktop keeps one — what is shown before the bus answers, and all
/// there is without it.
fn local_profile() -> Account {
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

    Account {
        object: None,
        uid: uid(),
        administrator: in_admin_group(&user),
        user,
        real_name,
        picture,
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

/// Where another account's picture is cut to before AccountsService copies
/// it in: the session's runtime directory, which only you can read.
fn scratch_face(user: &str) -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(|dir| PathBuf::from(dir).join(format!("otto-face-{user}.png")))
}

/// Cut `source` to a centered square, scale it to [`PICTURE_PX`] and write it
/// to `face`, which is then what AccountsService is handed. Your own goes to
/// `~/.face`, which keeps the desktops and display managers that read it
/// rather than the service in step with it.
fn write_face(source: &Path, face: &Path) -> Result<PathBuf, String> {
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

    // Written beside it and renamed over it, so a reader never finds half an
    // image.
    let partial = face.with_extension("otto-partial");
    std::fs::write(&partial, png.as_bytes())
        .and_then(|()| std::fs::rename(&partial, face))
        .map_err(|err| err.to_string())?;
    Ok(face.to_path_buf())
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

    fn run(program: &str, current: &str, new: &str) -> Result<(), PasswdError> {
        change_password(
            program,
            current,
            new,
            &AtomicBool::new(false),
            Duration::from_secs(10),
        )
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
    fn a_password_hashes_to_sha512_crypt() {
        use sha_crypt::{password_hash, PasswordVerifier, ShaCrypt};
        // A fresh password each run, not a literal: a hard-coded one reads
        // as a leaked secret to code scanning.
        let password: String = password_hash::try_generate_salt()
            .unwrap()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let hash = hash_password(&password).unwrap();
        assert!(hash.starts_with("$6$"));
        // glibc's crypt reads at most 16 characters of salt.
        let salt = hash.split('$').nth(3).unwrap();
        assert_eq!(salt.len(), 16);
        assert!(ShaCrypt::SHA512
            .verify_password(password.as_bytes(), hash.as_str())
            .is_ok());
        assert!(ShaCrypt::SHA512
            .verify_password(format!("{password}!").as_bytes(), hash.as_str())
            .is_err());
    }

    #[test]
    fn the_prompts_are_answered_in_order() {
        let program = fake_passwd(&scratch("ok"), ASKS);
        assert_eq!(run(&program, "old", "a-long-new-one"), Ok(()));
    }

    #[test]
    fn stopping_after_the_current_password_means_it_was_wrong() {
        let program = fake_passwd(&scratch("wrong"), ASKS);
        assert_eq!(
            run(&program, "nope", "a-long-new-one"),
            Err(PasswdError::WrongCurrent)
        );
    }

    #[test]
    fn a_quality_refusal_is_reported_in_its_own_words_and_not_answered_again() {
        let program = fake_passwd(&scratch("weak"), ASKS);
        assert_eq!(
            run(&program, "old", "short"),
            Err(PasswdError::Refused(
                "BAD PASSWORD: The password is shorter than 8 characters".into()
            ))
        );
    }

    #[test]
    fn a_fullwidth_colon_ends_a_prompt_too() {
        let program = fake_passwd(
            &scratch("fullwidth"),
            &ASKS
                .replace("Current password: ", "当前密码：")
                .replace("New password: ", "新的密码：")
                .replace("Retype new password: ", "重新输入新的密码："),
        );
        assert_eq!(run(&program, "old", "a-long-new-one"), Ok(()));
    }

    #[test]
    fn a_refusal_before_any_prompt_is_not_a_wrong_password() {
        let program = fake_passwd(
            &scratch("early"),
            "echo 'passwd: Authentication service cannot retrieve authentication info'; exit 1",
        );
        assert_eq!(
            run(&program, "old", "a-long-new-one"),
            Err(PasswdError::Refused(
                "passwd: Authentication service cannot retrieve authentication info".into()
            ))
        );
    }

    #[test]
    fn a_prompt_not_recognised_is_given_up_on() {
        let program = fake_passwd(&scratch("silent"), "printf 'Password? '; read cur; exit 0");
        let started = Instant::now();
        let outcome = change_password(
            &program,
            "old",
            "a-long-new-one",
            &AtomicBool::new(false),
            Duration::from_millis(300),
        );
        assert!(matches!(outcome, Err(PasswdError::Refused(_))));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn cancelling_stops_passwd_mid_conversation() {
        let program = fake_passwd(&scratch("cancel"), "printf 'Password? '; read cur; exit 0");
        let cancel = Arc::new(AtomicBool::new(false));
        let later = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            later.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let outcome = change_password(
            &program,
            "old",
            "a-long-new-one",
            &cancel,
            Duration::from_secs(60),
        );
        assert!(matches!(outcome, Err(PasswdError::Refused(_))));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_missing_program_is_a_refusal_not_a_hang() {
        assert!(matches!(
            run("/nonexistent/passwd", "a", "b"),
            Err(PasswdError::Refused(_))
        ));
    }

    #[test]
    fn only_the_password_fields_are_masked() {
        assert!(is_secret(CURRENT_ID) && is_secret(NEW_ID) && is_secret(CONFIRM_ID));
        assert!(!is_secret(NAME_ID) && !is_secret(PICTURE_ID));
        assert!(owns(NAME_ID) && !owns("dock.size"));
    }

    #[test]
    fn a_login_name_is_suggested_from_the_first_word() {
        assert_eq!(suggested_user_name("Ana López"), "ana");
        assert_eq!(suggested_user_name("  42Bob  Smith"), "bob");
        assert_eq!(suggested_user_name(""), "");
    }

    #[test]
    fn only_names_useradd_takes_are_valid() {
        assert!(valid_user_name("ana") && valid_user_name("_svc-1"));
        assert!(!valid_user_name("Ana") && !valid_user_name("1ana"));
        assert!(!valid_user_name("") && !valid_user_name(&"a".repeat(33)));
        assert!(!valid_user_name("ana lópez"));
    }

    #[test]
    fn each_sheet_walks_its_own_fields() {
        assert_eq!(Sheet::ChangePassword.fields().len(), 3);
        assert_eq!(Sheet::ResetPassword.fields(), &[NEW_ID, CONFIRM_ID]);
        assert_eq!(Sheet::AddUser.fields()[0], ADD_NAME_ID);
        assert!(Sheet::DeleteUser.fields().is_empty());
    }

    #[test]
    fn an_add_form_with_a_taken_name_is_refused_before_any_call() {
        let state = State {
            accounts: vec![Account {
                user: "ana".into(),
                ..Account::default()
            }],
            add_user: "ana".into(),
            new: "secret".into(),
            confirm: "secret".into(),
            ..State::default()
        };
        assert_eq!(
            problem(&state, Sheet::AddUser),
            Some(otto_kit::t!("settings-users-name-taken"))
        );
    }
}
