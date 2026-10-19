# Bug reports (`otto-msg doctor`)

**Status:** draft
**Related specs:** none

## Summary

`otto-msg doctor` puts together what a bug report about Otto needs in one Markdown
file, after the user has chosen and seen exactly what goes in it. It exists
because most reports arrive without the version, the graphics driver or the
log that would make them fixable, and gathering those by hand takes a dozen
commands.

## Goals

- One command gathers the operating system, graphics cards and drivers, the
  renderer, the screens, Otto's version and backend, the configuration files
  and the session logs.
- The user chooses which parts go in, and can read each part exactly as it
  will be written before writing it.
- Personal details are replaced by default, and the user can see the result.
- The report is useful when Otto has crashed or is not running.
- Nothing is sent anywhere by the tool.

## Non-Goals

- Uploading the report, or filing the issue for the user.
- Collecting anything beyond Otto and the system facts it depends on (other
  programs' logs, the journal, hardware inventories).
- Guaranteed anonymisation: redaction is a convenience, and the preview is
  how the user checks it.

## Behavior

**Sections.** A report is made of these sections, in this order:

| Section | Content | Default |
|---------|---------|---------|
| Operating system | `PRETTY_NAME` from os-release (or name and version), kernel release | included |
| Graphics | Each DRM card: PCI vendor and device IDs, vendor name for known vendors, kernel driver, whether it is the boot VGA device; the renderer the session draws with (API, vendor, renderer, version); why Vulkan fell back to OpenGL, if it did | included |
| Screens | Each output: name, mode size in pixels, refresh rate in Hz, scale, layout position, make and model, whether it is virtual | included |
| Otto | Otto's version and commit, backend, this tool's version, the configuration files the compositor reads | included |
| Configuration | Each configuration file, whole, in a fenced block | left out |
| Session logs | The last 2000 lines (configurable) of each log found (see below), without terminal escape codes; each says where it came from | left out |

Each section has a one-line summary shown in the list.

**The compositor.** Graphics renderer, Screens and Otto come from the running
compositor's `org.otto.Compositor.DebugInfo`. When the compositor is not on
the session bus, does not answer within three seconds, or is too old to have
the method, those parts are filled from the last `otto::summary` line in the
newest log that has one (in the order of *Finding the logs*, files only:
Otto's logs, a running Otto's stderr file, the `otto-session` logs). Each of
those sections then names the log and says the details are not live, and its
summary ends in `(from <log>)`. With no such line anywhere, they say so
("compositor not reachable", "compositor is running but too old to report
this") and the rest of the report is unchanged.

**The summary line.** The compositor logs its `DebugInfo` JSON at `info` as
`session summary: <JSON>` under the target `otto::summary`, once the renderer
and an output are up and again whenever the outputs change (added, removed,
mode, scale, position). An unchanged session logs it once. The reader ignores
colour codes and every other line.
The configuration section then lists the usual locations: the system file,
the user's file, and the local override files in the home folder.

**Finding the logs.** In this order, each file at most once:

1. The log file the compositor reports (`log_path`) and the same name with
   `.1`; then `otto.log` and `otto.log.1` in `$XDG_STATE_HOME/otto`.
2. For each running process of the same user named `otto` (by `comm`, or by
   its executable, also when that was replaced): where its stderr points. A
   regular file is read through the process's file descriptor. A socket, when
   the process's environment has `JOURNAL_STREAM`, is read with
   `journalctl --user _PID=<pid>`, falling back to the system journal. A
   terminal or anything else is skipped.
3. `session.log` and `session.log.1` from the `otto-session` login script,
   unless a running Otto's stderr already is that file.
4. `panic.log`.

The list summary names each source (`otto.log`, `stderr → session.log (pid
1234)`, `journal (pid 1234)`) with its line count.

**Redaction.** On by default. Replaces the home folder with `~` where it is a
whole path prefix, the login name with `<user>` and the host name with
`<host>` where they are whole words, and IPv4, IPv6 and MAC addresses with
`<ipv4>`, `<ipv6>` and `<mac>`. Loopback and unspecified addresses (`127.x`,
`0.0.0.0`, `::1`, `::`, an all-zero MAC) are kept. Clock times, version
numbers and code paths are not taken for addresses. Redaction applies to the
list summaries, the previews, the report and the issue link alike.

**Interactive chooser.** When both stdin and stdout are terminals and
`--print` is not given:

- A header reads `● ● Otto  doctor`, with the redaction state on the right.
- The sections are listed with `[x]` or `[ ]` and their summary, followed by
  a **Create report** entry. Up/Down and `j`/`k` move, Space toggles, `a`
  toggles all, Enter or `v` on a section opens its preview, `r` toggles
  redaction, `w` or Enter on **Create report** writes, `q`, Esc or Ctrl+C
  quits without writing.
- Writing with nothing selected is refused with a message.
- The preview shows the section's heading and body exactly as the report
  will carry them, scrollable with arrows, Page Up/Down, Space, `b`, `g` and
  `G`; `x` toggles the section; `q`, Esc or Enter returns to the list.
- A resize redraws the screen at the new size. Long lines are cut with `…`.
- Content from files cannot move the cursor or change colours: control
  characters are shown as a replacement character.
- On exit, by any path including a panic, the terminal is back in its
  original mode and on the main screen.

**Writing.** The report goes to `$XDG_STATE_HOME/otto/otto-report-YYYYMMDD-HHMMSS.md`
(local time), or to `--output`. It starts with a title, the tool version, the
time and whether it is redacted, then has one `## <section>` per included
section. After writing, the tool copies the report to the clipboard with
`wl-copy` when a Wayland session and `wl-copy` are present (failure is
silent), prints the path, and prints a link to
`https://github.com/nongio/otto/issues/new` whose body has prompts for what
happened and the steps, the summaries of the included system sections, and a
reminder to attach the file. The link is at most 6000 characters; summary
lines are dropped from the end to fit.

**Non-interactive.** `--print`, or stdout not a terminal: the report with the
default selection (or all with `--all`) is written to stdout, and nothing
else happens. Stdin not a terminal (and stdout a terminal): the report is
written to the file as above, without the chooser.

**Options.** `--all`, `--no-redact`, `--print`, `--output <path>`,
`--lines <n>`, `--no-clipboard`, `--help`, `--version`. `--print` with
`--output` is an error. An unknown option prints the usage and exits with
status 2.

## Constraints & Edge Cases

- A log can be very large; only the last 8 MiB of each is read, and the first,
  partial line of that window is dropped.
- A configuration file containing backtick fences must not end its code block
  early: the fence is longer than any backtick run in the file.
- A machine with no DRM cards (a container, some VMs) says so in the Graphics
  section.
- A frozen compositor must not hang the tool: the D-Bus wait is bounded.
- The tool needs no compositor, no Wayland connection and no GPU.

## Rationale

- Configuration and logs are off by default because they can hold autostart
  commands, paths and names a user may not want to publish; the system
  sections are what almost every report needs and hold nothing personal once
  redacted.
- The preview shows the redacted text rather than the original so the user
  checks what will actually be shared.
- The report is attached rather than pasted into the issue: logs exceed what
  a pre-filled link can carry, and a file is easy to read and search.
- The interface is hand-written over termios and ANSI escapes rather than a
  terminal UI library, to keep the dependency set small.

## Open Questions

- Should the journal (`journalctl --user -b`) be offered as a section for
  sessions not started through `otto-session`?
