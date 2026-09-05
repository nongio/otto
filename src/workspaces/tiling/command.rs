//! The i3 command language, parsed.
//!
//! One hand-written parser, no dependency, shared by the D-Bus `RunCommand`
//! method, the `otto-msg` CLI and the headless tests — so an `i3-msg` or
//! `swaymsg` script ports to Otto with a rename (see
//! `docs/developer/tiling-plan.md`, *Command language*).
//!
//! Nothing here touches the compositor: [`parse`] turns text into
//! [`Command`]s and [`crate::shell::commands`] runs them through the same
//! handlers the keybindings use.
//!
//! Commands are separated by `;`. Errors carry the byte offset of the
//! offending token so `otto-msg` can point at it the way `swaymsg` does.

use super::tree::{Axis, Direction};

/// Which workspace `workspace …` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceTarget {
    /// A workspace number, 1-based as i3 writes it.
    Number(usize),
    Next,
    Prev,
}

/// The three-state argument i3 gives every toggleable command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Toggle,
    Enable,
    Disable,
}

/// `split h|v|toggle`, and `layout splith|splitv|toggle split`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisArg {
    /// An explicit axis: `h`/`splith` is [`Axis::Row`], `v`/`splitv` is
    /// [`Axis::Column`].
    Axis(Axis),
    /// Flip whatever is in force.
    Toggle,
}

/// How much a `resize` step moves, before it is resolved against the
/// container it applies to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Amount {
    /// A percentage of the container's extent — i3's `ppt`.
    Ppt(f32),
    /// Logical pixels, resolved against the container's extent at run time.
    Px(i32),
}

/// Which gap `gaps …` sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapScope {
    Inner,
    Outer,
}

/// How far a `gaps` command reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapTarget {
    /// Override the focused workspace only. The default, as in sway.
    Current,
    /// Set the session default and drop every per-workspace override.
    All,
}

/// One parsed command.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `focus left|right|up|down`
    Focus(Direction),
    /// `focus parent`
    FocusParent,
    /// `focus child`
    FocusChild,
    /// `move left|right|up|down`
    Move(Direction),
    /// `move container to workspace <n>`
    MoveToWorkspace(usize),
    /// `workspace <n|next|prev>`
    Workspace(WorkspaceTarget),
    /// `split h|v|toggle`
    Split(AxisArg),
    /// `layout splith|splitv|toggle split`
    Layout(AxisArg),
    /// `resize grow|shrink width|height <n> px|ppt`
    Resize {
        axis: Axis,
        grow: bool,
        amount: Amount,
    },
    /// `fullscreen [toggle]`
    Fullscreen,
    /// `kill`
    Kill,
    /// `tiling toggle|enable|disable`
    Tiling(Toggle),
    /// `gaps inner|outer <n> [current|all]`
    Gaps {
        scope: GapScope,
        target: GapTarget,
        amount: i32,
    },
}

/// A command that could not be parsed, or that Otto does not implement yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset into the whole command string of the token at fault.
    pub offset: usize,
    /// The message, in the shape `swaymsg` prints.
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parse a whole `;`-separated command string.
///
/// An empty string, or one holding only separators and whitespace, parses to
/// no commands rather than to an error — that is what `swaymsg ''` does.
pub fn parse(text: &str) -> Result<Vec<Command>, ParseError> {
    let mut out = Vec::new();
    for (offset, segment) in split_commands(text) {
        let tokens = tokenize(segment, offset);
        if tokens.is_empty() {
            continue;
        }
        out.push(parse_one(&mut Cursor::new(
            &tokens,
            offset + segment.len(),
        ))?);
    }
    Ok(out)
}

/// Split on `;`, keeping each segment's byte offset in the original string.
fn split_commands(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, ch) in text.char_indices() {
        if ch == ';' {
            out.push((start, &text[start..i]));
            start = i + ch.len_utf8();
        }
    }
    out.push((start, &text[start..]));
    out
}

/// One whitespace-separated word, and where it starts in the whole string.
type Token<'a> = (usize, &'a str);

fn tokenize(segment: &str, base: usize) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut word_start: Option<usize> = None;
    for (i, ch) in segment.char_indices() {
        if ch.is_whitespace() {
            if let Some(start) = word_start.take() {
                out.push((base + start, &segment[start..i]));
            }
        } else if word_start.is_none() {
            word_start = Some(i);
        }
    }
    if let Some(start) = word_start {
        out.push((base + start, &segment[start..]));
    }
    out
}

/// A read cursor over one command's tokens.
struct Cursor<'a> {
    tokens: &'a [Token<'a>],
    at: usize,
    /// Offset just past the command, used when an argument is missing.
    end: usize,
}

impl<'a> Cursor<'a> {
    fn new(tokens: &'a [Token<'a>], end: usize) -> Self {
        Cursor { tokens, at: 0, end }
    }

    /// The next word, without consuming it.
    fn peek(&self) -> Option<&'a str> {
        self.tokens.get(self.at).map(|(_, word)| *word)
    }

    /// Consume the next word.
    fn next(&mut self) -> Option<Token<'a>> {
        let token = self.tokens.get(self.at).copied();
        if token.is_some() {
            self.at += 1;
        }
        token
    }

    /// Where the token at `at` starts, or the end of the command when there
    /// is none left — so "you forgot an argument" points past the last word
    /// rather than at it.
    fn offset(&self) -> usize {
        self.tokens
            .get(self.at)
            .map(|(offset, _)| *offset)
            .unwrap_or(self.end)
    }

    fn is_done(&self) -> bool {
        self.at >= self.tokens.len()
    }
}

/// `Unknown/invalid command 'x'` — the shape `swaymsg` prints.
fn unknown(offset: usize, word: &str) -> ParseError {
    ParseError {
        offset,
        message: format!("Unknown/invalid command '{word}'"),
    }
}

/// `Invalid focus command (expected …)` — the shape `swaymsg` prints for a
/// command it knows with an argument it does not.
fn invalid(offset: usize, command: &str, expected: &str) -> ParseError {
    ParseError {
        offset,
        message: format!("Invalid {command} command (expected {expected})"),
    }
}

/// Parsed fine, but Otto has not built it yet. Distinct from a typo, and said
/// so, because silently ignoring it would leave a script believing it worked.
fn unsupported(offset: usize, what: &str) -> ParseError {
    ParseError {
        offset,
        message: format!("Otto does not support '{what}' yet"),
    }
}

fn parse_one(cursor: &mut Cursor<'_>) -> Result<Command, ParseError> {
    let (offset, word) = cursor.next().expect("a command has at least one token");
    let command = match word {
        "focus" => parse_focus(cursor)?,
        "move" => parse_move(cursor)?,
        "workspace" => parse_workspace(cursor)?,
        "split" => Command::Split(parse_split_arg(cursor)?),
        "layout" => parse_layout(cursor)?,
        "resize" => parse_resize(cursor)?,
        "floating" => {
            // The floating layer is a later branch; say so rather than
            // pretending the window moved.
            let arg = cursor.peek().unwrap_or("");
            return Err(unsupported(offset, &join("floating", arg)));
        }
        "fullscreen" => parse_fullscreen(cursor, offset)?,
        "kill" => Command::Kill,
        "tiling" => Command::Tiling(parse_toggle(cursor, "tiling")?),
        "gaps" => parse_gaps(cursor)?,
        other => return Err(unknown(offset, other)),
    };
    if !cursor.is_done() {
        let (offset, extra) = cursor.next().expect("not done");
        return Err(unknown(offset, extra));
    }
    Ok(command)
}

fn join(command: &str, arg: &str) -> String {
    if arg.is_empty() {
        command.to_string()
    } else {
        format!("{command} {arg}")
    }
}

fn direction(word: &str) -> Option<Direction> {
    match word {
        "left" => Some(Direction::Left),
        "right" => Some(Direction::Right),
        "up" => Some(Direction::Up),
        "down" => Some(Direction::Down),
        _ => None,
    }
}

fn parse_focus(cursor: &mut Cursor<'_>) -> Result<Command, ParseError> {
    let offset = cursor.offset();
    let Some((_, word)) = cursor.next() else {
        return Err(invalid(offset, "focus", "left|right|up|down|parent|child"));
    };
    if let Some(dir) = direction(word) {
        return Ok(Command::Focus(dir));
    }
    match word {
        "parent" => Ok(Command::FocusParent),
        "child" => Ok(Command::FocusChild),
        // i3's second focus axis — the floating layer, which Otto has not
        // built yet.
        "mode_toggle" | "floating" | "tiling" => Err(unsupported(offset, &join("focus", word))),
        _ => Err(invalid(offset, "focus", "left|right|up|down|parent|child")),
    }
}

fn parse_move(cursor: &mut Cursor<'_>) -> Result<Command, ParseError> {
    let offset = cursor.offset();
    let Some((_, word)) = cursor.next() else {
        return Err(invalid(
            offset,
            "move",
            "left|right|up|down, or 'container to workspace <n>'",
        ));
    };
    if let Some(dir) = direction(word) {
        return Ok(Command::Move(dir));
    }
    // `move container to workspace 3`, and i3's shorter spellings of it.
    let mut expect_to = matches!(word, "container" | "window");
    if word == "to" {
        expect_to = false;
    } else if !expect_to {
        return Err(invalid(
            offset,
            "move",
            "left|right|up|down, or 'container to workspace <n>'",
        ));
    }
    if expect_to {
        let to_offset = cursor.offset();
        match cursor.next() {
            Some((_, "to")) => {}
            _ => return Err(invalid(to_offset, "move", "'to workspace <n>'")),
        }
    }
    let ws_offset = cursor.offset();
    match cursor.next() {
        Some((_, "workspace")) => {}
        Some((offset, "output")) => return Err(unsupported(offset, "move … to output")),
        _ => return Err(invalid(ws_offset, "move", "'to workspace <n>'")),
    }
    // i3 allows `workspace number 3`; the word is noise here.
    if cursor.peek() == Some("number") {
        cursor.next();
    }
    let n_offset = cursor.offset();
    let Some((_, n)) = cursor.next() else {
        return Err(invalid(n_offset, "move", "a workspace number"));
    };
    match n.parse::<usize>() {
        Ok(n) if n >= 1 => Ok(Command::MoveToWorkspace(n)),
        _ => Err(invalid(n_offset, "move", "a workspace number from 1")),
    }
}

fn parse_workspace(cursor: &mut Cursor<'_>) -> Result<Command, ParseError> {
    let offset = cursor.offset();
    if cursor.peek() == Some("number") {
        cursor.next();
    }
    let Some((word_offset, word)) = cursor.next() else {
        return Err(invalid(offset, "workspace", "<n>|next|prev"));
    };
    match word {
        "next" => Ok(Command::Workspace(WorkspaceTarget::Next)),
        "prev" | "previous" => Ok(Command::Workspace(WorkspaceTarget::Prev)),
        "back_and_forth" | "next_on_output" | "prev_on_output" => {
            Err(unsupported(word_offset, &join("workspace", word)))
        }
        other => match other.parse::<usize>() {
            Ok(n) if n >= 1 => Ok(Command::Workspace(WorkspaceTarget::Number(n))),
            // A named workspace is a later branch: Otto's workspaces are
            // named per output, and `workspace <name>` would have to create
            // one before it could switch to it.
            _ => Err(invalid(word_offset, "workspace", "<n>|next|prev")),
        },
    }
}

fn parse_split_arg(cursor: &mut Cursor<'_>) -> Result<AxisArg, ParseError> {
    let offset = cursor.offset();
    let Some((_, word)) = cursor.next() else {
        return Err(invalid(offset, "split", "h|v|toggle"));
    };
    match word {
        "h" | "horizontal" => Ok(AxisArg::Axis(Axis::Row)),
        "v" | "vertical" => Ok(AxisArg::Axis(Axis::Column)),
        "t" | "toggle" => Ok(AxisArg::Toggle),
        "none" => Err(unsupported(offset, "split none")),
        _ => Err(invalid(offset, "split", "h|v|toggle")),
    }
}

fn parse_layout(cursor: &mut Cursor<'_>) -> Result<Command, ParseError> {
    let offset = cursor.offset();
    let Some((_, word)) = cursor.next() else {
        return Err(invalid(offset, "layout", "splith|splitv|toggle split"));
    };
    match word {
        "splith" => Ok(Command::Layout(AxisArg::Axis(Axis::Row))),
        "splitv" => Ok(Command::Layout(AxisArg::Axis(Axis::Column))),
        // A tabbed or stacked container is drawn, not just laid out; until
        // the title strip exists, saying no is the honest answer.
        "tabbed" | "stacking" | "stacked" | "default" => {
            Err(unsupported(offset, &join("layout", word)))
        }
        "toggle" => match cursor.peek() {
            None | Some("split") => {
                cursor.next();
                Ok(Command::Layout(AxisArg::Toggle))
            }
            Some(_) => {
                let (offset, word) = cursor.next().expect("peeked");
                Err(unsupported(offset, &format!("layout toggle {word}")))
            }
        },
        _ => Err(invalid(offset, "layout", "splith|splitv|toggle split")),
    }
}

fn parse_resize(cursor: &mut Cursor<'_>) -> Result<Command, ParseError> {
    let offset = cursor.offset();
    let Some((_, word)) = cursor.next() else {
        return Err(invalid(
            offset,
            "resize",
            "grow|shrink width|height <n> px|ppt",
        ));
    };
    let grow = match word {
        "grow" => true,
        "shrink" => false,
        "set" => return Err(unsupported(offset, "resize set")),
        _ => {
            return Err(invalid(
                offset,
                "resize",
                "grow|shrink width|height <n> px|ppt",
            ))
        }
    };
    let axis_offset = cursor.offset();
    let axis = match cursor.next() {
        Some((_, "width")) => Axis::Row,
        Some((_, "height")) => Axis::Column,
        _ => return Err(invalid(axis_offset, "resize", "width|height")),
    };
    let amount = parse_amount(cursor, "resize")?;
    // i3's `or <n> ppt` fallback: the second figure only matters to a tiler
    // that resizes floating windows in pixels and tiled ones in points.
    // Otto's tree is fractions throughout, so it is accepted and dropped.
    if cursor.peek() == Some("or") {
        cursor.next();
        parse_amount(cursor, "resize")?;
    }
    Ok(Command::Resize { axis, grow, amount })
}

fn parse_amount(cursor: &mut Cursor<'_>, command: &str) -> Result<Amount, ParseError> {
    let offset = cursor.offset();
    let Some((_, n)) = cursor.next() else {
        return Err(invalid(offset, command, "an amount"));
    };
    let Ok(value) = n.parse::<f32>() else {
        return Err(invalid(offset, command, "an amount"));
    };
    match cursor.peek() {
        Some("ppt") => {
            cursor.next();
            Ok(Amount::Ppt(value))
        }
        Some("px") => {
            cursor.next();
            Ok(Amount::Px(value as i32))
        }
        // i3 defaults a bare figure to ppt for a tiled window, which is the
        // only kind the tree holds.
        _ => Ok(Amount::Ppt(value)),
    }
}

fn parse_fullscreen(cursor: &mut Cursor<'_>, offset: usize) -> Result<Command, ParseError> {
    match cursor.peek() {
        None | Some("toggle") => {
            cursor.next();
            Ok(Command::Fullscreen)
        }
        Some("global") => Err(unsupported(cursor.offset(), "fullscreen global")),
        Some(_) => {
            let (word_offset, word) = cursor.next().expect("peeked");
            let _ = offset;
            Err(invalid(
                word_offset,
                "fullscreen",
                &format!("toggle, not '{word}'"),
            ))
        }
    }
}

fn parse_toggle(cursor: &mut Cursor<'_>, command: &str) -> Result<Toggle, ParseError> {
    let offset = cursor.offset();
    match cursor.peek() {
        None | Some("toggle") => {
            cursor.next();
            Ok(Toggle::Toggle)
        }
        Some("enable") | Some("on") => {
            cursor.next();
            Ok(Toggle::Enable)
        }
        Some("disable") | Some("off") => {
            cursor.next();
            Ok(Toggle::Disable)
        }
        Some(_) => Err(invalid(offset, command, "toggle|enable|disable")),
    }
}

fn parse_gaps(cursor: &mut Cursor<'_>) -> Result<Command, ParseError> {
    let offset = cursor.offset();
    let scope = match cursor.next() {
        Some((_, "inner")) => GapScope::Inner,
        Some((_, "outer")) => GapScope::Outer,
        Some((offset, "horizontal"))
        | Some((offset, "vertical"))
        | Some((offset, "top"))
        | Some((offset, "bottom"))
        | Some((offset, "left"))
        | Some((offset, "right")) => return Err(unsupported(offset, "per-edge gaps")),
        _ => return Err(invalid(offset, "gaps", "inner|outer <n> [current|all]")),
    };
    // sway writes the reach before the amount (`gaps inner all set 10`) and
    // Otto's own shape puts it after (`gaps inner 10 all`). Both read, and
    // the last word wins; `set` is noise either way.
    let mut target = GapTarget::Current;
    if let Some(read) = gap_target(cursor.peek()) {
        cursor.next();
        target = read;
    }
    if cursor.peek() == Some("set") {
        cursor.next();
    }
    let n_offset = cursor.offset();
    let Some((_, n)) = cursor.next() else {
        return Err(invalid(n_offset, "gaps", "inner|outer <n> [current|all]"));
    };
    let Ok(amount) = n.parse::<i32>() else {
        return Err(invalid(n_offset, "gaps", "a gap in logical pixels"));
    };
    if amount < 0 {
        return Err(invalid(n_offset, "gaps", "a gap in logical pixels"));
    }
    let trailing_offset = cursor.offset();
    if let Some(word) = cursor.peek() {
        match gap_target(Some(word)) {
            Some(read) => {
                cursor.next();
                target = read;
            }
            None => return Err(invalid(trailing_offset, "gaps", "current|all")),
        }
    }
    Ok(Command::Gaps {
        scope,
        target,
        amount,
    })
}

fn gap_target(word: Option<&str>) -> Option<GapTarget> {
    match word {
        Some("current") => Some(GapTarget::Current),
        Some("all") => Some(GapTarget::All),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Command {
        let mut commands = parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(commands.len(), 1, "{text} is one command");
        commands.pop().unwrap()
    }

    fn error(text: &str) -> ParseError {
        parse(text).expect_err(text)
    }

    #[test]
    fn focus_takes_the_four_directions() {
        assert_eq!(one("focus left"), Command::Focus(Direction::Left));
        assert_eq!(one("focus right"), Command::Focus(Direction::Right));
        assert_eq!(one("focus up"), Command::Focus(Direction::Up));
        assert_eq!(one("focus down"), Command::Focus(Direction::Down));
    }

    #[test]
    fn focus_walks_the_tree() {
        assert_eq!(one("focus parent"), Command::FocusParent);
        assert_eq!(one("focus child"), Command::FocusChild);
    }

    #[test]
    fn move_takes_directions_and_a_workspace() {
        assert_eq!(one("move left"), Command::Move(Direction::Left));
        assert_eq!(
            one("move container to workspace 2"),
            Command::MoveToWorkspace(2)
        );
        assert_eq!(one("move to workspace 3"), Command::MoveToWorkspace(3));
        assert_eq!(
            one("move window to workspace number 4"),
            Command::MoveToWorkspace(4)
        );
    }

    #[test]
    fn workspace_takes_a_number_or_a_direction() {
        assert_eq!(
            one("workspace 3"),
            Command::Workspace(WorkspaceTarget::Number(3))
        );
        assert_eq!(
            one("workspace number 3"),
            Command::Workspace(WorkspaceTarget::Number(3))
        );
        assert_eq!(
            one("workspace next"),
            Command::Workspace(WorkspaceTarget::Next)
        );
        assert_eq!(
            one("workspace prev"),
            Command::Workspace(WorkspaceTarget::Prev)
        );
    }

    #[test]
    fn split_and_layout_carry_an_axis() {
        assert_eq!(one("split h"), Command::Split(AxisArg::Axis(Axis::Row)));
        assert_eq!(one("split v"), Command::Split(AxisArg::Axis(Axis::Column)));
        assert_eq!(one("split toggle"), Command::Split(AxisArg::Toggle));
        assert_eq!(
            one("layout splith"),
            Command::Layout(AxisArg::Axis(Axis::Row))
        );
        assert_eq!(
            one("layout splitv"),
            Command::Layout(AxisArg::Axis(Axis::Column))
        );
        assert_eq!(one("layout toggle split"), Command::Layout(AxisArg::Toggle));
        assert_eq!(one("layout toggle"), Command::Layout(AxisArg::Toggle));
    }

    #[test]
    fn resize_reads_both_units() {
        assert_eq!(
            one("resize grow width 10 ppt"),
            Command::Resize {
                axis: Axis::Row,
                grow: true,
                amount: Amount::Ppt(10.0)
            }
        );
        assert_eq!(
            one("resize shrink height 40 px"),
            Command::Resize {
                axis: Axis::Column,
                grow: false,
                amount: Amount::Px(40)
            }
        );
        // A bare figure is ppt, as it is for a tiled window in i3.
        assert_eq!(
            one("resize grow width 5"),
            Command::Resize {
                axis: Axis::Row,
                grow: true,
                amount: Amount::Ppt(5.0)
            }
        );
    }

    #[test]
    fn the_or_ppt_fallback_is_accepted_and_dropped() {
        assert_eq!(
            one("resize grow width 10 px or 5 ppt"),
            Command::Resize {
                axis: Axis::Row,
                grow: true,
                amount: Amount::Px(10)
            }
        );
    }

    #[test]
    fn the_standalone_commands_parse() {
        assert_eq!(one("kill"), Command::Kill);
        assert_eq!(one("fullscreen"), Command::Fullscreen);
        assert_eq!(one("fullscreen toggle"), Command::Fullscreen);
        assert_eq!(one("tiling toggle"), Command::Tiling(Toggle::Toggle));
        assert_eq!(one("tiling enable"), Command::Tiling(Toggle::Enable));
        assert_eq!(one("tiling disable"), Command::Tiling(Toggle::Disable));
        assert_eq!(one("tiling"), Command::Tiling(Toggle::Toggle));
    }

    #[test]
    fn gaps_reaches_this_workspace_unless_told_otherwise() {
        // The bare form is sway's: this workspace only.
        assert_eq!(
            one("gaps inner 12"),
            Command::Gaps {
                scope: GapScope::Inner,
                target: GapTarget::Current,
                amount: 12
            }
        );
        assert_eq!(
            one("gaps inner 12 current"),
            Command::Gaps {
                scope: GapScope::Inner,
                target: GapTarget::Current,
                amount: 12
            }
        );
        assert_eq!(
            one("gaps outer 4 all"),
            Command::Gaps {
                scope: GapScope::Outer,
                target: GapTarget::All,
                amount: 4
            }
        );
        // sway's own word order reads the same.
        assert_eq!(
            one("gaps outer all set 4"),
            Command::Gaps {
                scope: GapScope::Outer,
                target: GapTarget::All,
                amount: 4
            }
        );
    }

    #[test]
    fn a_semicolon_separates_commands() {
        assert_eq!(
            parse("focus left; move right ; split v").unwrap(),
            vec![
                Command::Focus(Direction::Left),
                Command::Move(Direction::Right),
                Command::Split(AxisArg::Axis(Axis::Column)),
            ]
        );
    }

    #[test]
    fn empty_input_is_no_commands() {
        assert!(parse("").unwrap().is_empty());
        assert!(parse("  ;  ; ").unwrap().is_empty());
    }

    // ── Errors ───────────────────────────────────────────────────────────

    #[test]
    fn an_unknown_command_points_at_its_first_word() {
        let error = error("focus left; frobnicate");
        assert_eq!(error.offset, 12);
        assert_eq!(error.message, "Unknown/invalid command 'frobnicate'");
    }

    #[test]
    fn a_bad_argument_points_at_the_argument() {
        let error = error("focus sideways");
        assert_eq!(error.offset, 6);
        assert_eq!(
            error.message,
            "Invalid focus command (expected left|right|up|down|parent|child)"
        );
    }

    #[test]
    fn a_missing_argument_points_past_the_command() {
        assert_eq!(error("focus").offset, 5, "just past `focus`");
        assert_eq!(error("resize grow width").offset, 17);
    }

    #[test]
    fn trailing_junk_is_an_error_at_the_junk() {
        let error = error("focus left up");
        assert_eq!(error.offset, 11);
        assert_eq!(error.message, "Unknown/invalid command 'up'");
    }

    #[test]
    fn the_unsupported_commands_say_so() {
        for (text, offset) in [
            ("floating toggle", 0),
            ("layout tabbed", 7),
            ("layout stacking", 7),
            ("focus mode_toggle", 6),
            ("resize set width 30 ppt", 7),
        ] {
            let error = error(text);
            assert_eq!(error.offset, offset, "{text}");
            assert!(
                error.message.contains("does not support"),
                "{text}: {}",
                error.message
            );
        }
    }

    #[test]
    fn an_offset_is_a_byte_index_into_the_whole_string() {
        let error = error("split v; layout tabbed");
        assert_eq!(&"split v; layout tabbed"[error.offset..], "tabbed");
    }
}
