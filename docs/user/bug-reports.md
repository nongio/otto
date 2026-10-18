# Reporting a bug

`otto-msg doctor` gathers what a bug report about Otto needs, lets you choose what
to share, and writes it as one Markdown file you attach to an issue. It never
uploads anything: the file stays on your computer until you attach it.

## Writing a report

Open a terminal and run:

```sh
otto-msg doctor
```

It lists the parts of a report, each with a one-line summary:

| Section | What it holds | Included by default |
|---------|---------------|---------------------|
| Operating system | Your distribution and kernel version | yes |
| Graphics | Each graphics card, its kernel driver, and the renderer Otto draws with | yes |
| Screens | Each screen's name, resolution, refresh rate, scale and position | yes |
| Otto | Otto's version, the backend it runs on, and the configuration files it reads | yes |
| Configuration | Your configuration files, whole | no |
| Session logs | The end of Otto's log for this session and the previous one, wherever a running Otto writes its output, and the crash log | no |

The configuration and the logs are left out until you tick them, because they
can hold things you may not want to share, such as the programs you start at
login. When the problem is a crash or a freeze, the session logs are usually
what makes it fixable: the previous session's log is normally the one that
went wrong.

The logs are looked for in this order, and the list says where each one came
from:

1. Otto's own log, `otto.log`, and the previous session's `otto.log.1`.
2. Where a running Otto sends its output: a file, or the journal when Otto
   was started as a systemd service (read with `journalctl`).
3. `session.log` and `session.log.1`, which the `otto-session` login script
   keeps.
4. `panic.log`, every recorded crash.

The keys:

| Key | Does |
|-----|------|
| `↑` `↓` or `j` `k` | Move between sections |
| `Space` | Tick or untick a section |
| `a` | Tick or untick everything |
| `Enter` or `v` | Preview a section: exactly the text the report will contain |
| `r` | Turn redaction on or off |
| `w`, or `Enter` on **Create report** | Write the report |
| `q` or `Esc` | Leave without writing anything |

In a preview, the arrow keys, `Space` and `b` scroll, `x` ticks or unticks
the section, and `q` goes back to the list.

The report is written to `~/.local/state/otto/otto-report-<date>-<time>.md`
(under `$XDG_STATE_HOME` when that is set) and copied to the clipboard when
`wl-copy` is installed. `otto-msg doctor` then prints a link that opens a new issue
on GitHub with the summary filled in. Describe what happened, then drag the
report file into the issue to attach it.

## Redaction

Redaction is on unless you turn it off. It replaces:

- your home folder with `~`,
- your user name with `<user>` and your computer's name with `<host>`,
- IP addresses with `<ipv4>` and `<ipv6>`, and network hardware addresses with
  `<mac>`. Loopback addresses such as `127.0.0.1` are kept, because they say
  nothing about your network.

The preview always shows the text after redaction, so what you read is what
you share. Redaction works on patterns, so it is worth a look through a log
before you attach it: a Wi-Fi network name or an email address in a log line
is not replaced.

## When Otto is not running

`otto-msg doctor` asks the running Otto for its version, renderer and screens. When
it cannot, because Otto crashed or you are in another session, those sections
say "compositor not reachable" and the report still has everything else. The
configuration section then lists the usual files, and the session logs from
the last run are still there to include.

## Options

| Option | Does |
|--------|------|
| `-a`, `--all` | Include every section |
| `--no-redact` | Start with redaction off |
| `-p`, `--print` | Print the report instead of choosing, with the default sections (or every one with `--all`) |
| `-o`, `--output <path>` | Write the report to this file |
| `-n`, `--lines <n>` | Keep this many lines from the end of each log (2000 by default) |
| `--no-clipboard` | Do not copy the report to the clipboard |
| `-V`, `--version` | Print the version |
| `-h`, `--help` | Print the options |

When its output is not a terminal, `otto-msg doctor` prints the report, so
`otto-msg doctor --all > report.md` works too.

## See also

- [Troubleshooting](troubleshooting.md): where the logs are and the most common
  failures.
