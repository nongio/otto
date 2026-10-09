# Later, and again

Remind the person of something at a time, or do a piece of work later, once or
on a schedule. Otto has no scheduler of its own: systemd already is one, so use
it. Everything here is an ordinary systemd user unit, named `otto-later-<name>`,
which the person can list, read and remove with standard commands.

## First, check systemd

```sh
systemctl --user is-system-running
```

`running` or `degraded` means it is there; use the recipes below. Anything
else, or no `systemctl`, means this system cannot schedule anything for you.
Say so plainly, and offer the moment's version instead: a notification now.

## Pick the recipe

| They want | Recipe |
|---|---|
| A nudge later today ("in 20 minutes", "at 5") | [Soon](#soon) |
| A nudge on a date, or every week, month, morning | [On a date, or again and again](#on-a-date-or-again-and-again) |
| Work done later: a check, a summary, a report | [Work, not a nudge](#work-not-a-nudge) |

Pick a short name from what it is for, like `call-dentist` or `weekly-stats`.
Before writing it, ask with the question tool when the time or the repeat is
left open ("Tuesday at 9?", "every weekday or every day?").

## Soon

For today only. It is gone if the computer restarts, which is fine for a nudge
in an hour.

```sh
systemd-run --user --unit=otto-later-tea --on-active=20m \
  /usr/bin/notify-send -a "Otto" "Tea" "It has been twenty minutes."
systemd-run --user --unit=otto-later-call --on-calendar='17:00' \
  /usr/bin/notify-send -a "Otto" "Call the bank" "Before they close at half past."
```

`--on-active` takes `20m`, `1h30m`, `90s`; `--on-calendar` takes a time of day.

## On a date, or again and again

Two files, which survive restarts. `Persistent=true` means that if the computer
was off when it was due, it fires once as soon as it is back.

```sh
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/otto-later-plants.timer <<'EOF'
[Unit]
Description=Otto later: water the plants

[Timer]
OnCalendar=Sat 10:00
Persistent=true

[Install]
WantedBy=timers.target
EOF
cat > ~/.config/systemd/user/otto-later-plants.service <<'EOF'
[Unit]
Description=Otto later: water the plants

[Service]
Type=oneshot
ExecStart=/usr/bin/notify-send -a "Otto" "Water the plants" "The basil on the balcony too."
EOF
systemctl --user daemon-reload
systemctl --user enable --now otto-later-plants.timer
```

`OnCalendar=` says when:

| When | `OnCalendar=` |
|---|---|
| Once, on a date | `2026-10-14 09:00` |
| Every day | `*-*-* 08:30` |
| Weekdays | `Mon..Fri 09:00` |
| Every Saturday | `Sat 10:00` |
| First of the month | `*-*-01 09:00` |
| Every four hours | `*-*-* 00/4:00` |

Check one before writing it. This prints the next times it fires:

```sh
systemd-analyze calendar --iterations=3 'Mon..Fri 09:00'
```

**A one-off cleans up after itself.** For a single date, add this line to the
`.service`, under `ExecStart=`, so the files go once it has fired:

```ini
ExecStartPost=/bin/sh -c 'systemctl --user disable otto-later-NAME.timer; rm -f %h/.config/systemd/user/otto-later-NAME.*'
```

## Work, not a nudge

The same two files. The service runs your own command for working without
anyone watching, and the person gets a notification when it is done. With
Claude Code that is `claude -p`; another agent has its own.

The run starts from nothing. It does not remember this conversation and it
cannot ask anyone anything, so:

- **Write the message so it stands alone.** Say what to do, where, and what a
  good result looks like, with the paths and names spelled out.
- **Allow only the tools it needs**, and nothing that changes things it was not
  asked to change.
- **Use full paths.** systemd does not have the person's `PATH`. Find a command's
  path with `command -v claude`.

```ini
# ~/.config/systemd/user/otto-later-weekly-stats.service
[Unit]
Description=Otto later: weekly Otto stats

[Service]
Type=oneshot
WorkingDirectory=%h/dev/otto
ExecStart=%h/.local/bin/claude -p --allowedTools "Bash(gh api *) Read" \
  "Compare this week's GitHub stars, clones and release downloads for nongio/otto with last week. Five lines at most."
StandardOutput=truncate:%h/.local/state/otto/later/weekly-stats.md
ExecStopPost=/bin/sh -c '/usr/bin/notify-send -a "Otto" "Weekly stats: $$SERVICE_RESULT" "In ~/.local/state/otto/later/weekly-stats.md"'
```

Run `mkdir -p ~/.local/state/otto/later` before the first run.
`$SERVICE_RESULT` is `success`, or says how it failed, so the person hears about
a failure too.

## See, run, remove

Tell the person these work without you:

```sh
systemctl --user list-timers 'otto-later-*'          # what is set, and when it next fires
systemctl --user start otto-later-NAME.service       # run it now, to try it
journalctl --user -u otto-later-NAME.service         # what happened last time
systemctl --user disable --now otto-later-NAME.timer # stop it
rm ~/.config/systemd/user/otto-later-NAME.*          # and forget it
systemctl --user daemon-reload
```

After writing a timer, show it with `list-timers` so the person sees the next
time it fires. Test a work recipe with `start` and read the result before you
leave it to the schedule.
