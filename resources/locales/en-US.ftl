# Otto — American English
#
# A sparse overlay on en-GB. Only keys whose spelling or format actually
# differs belong here; everything else falls through the bundle chain to
# en-GB.ftl. Do not copy an unchanged string in — a duplicate silently stops
# tracking future edits to the source.


## Spelling

settings-accent-colour = Accent color
settings-colour-scheme = Color scheme
settings-background-colour = Background color
greeter-status-no-match = Fingerprint not recognized
lock-status-no-match = Fingerprint not recognized
launcher-ask-cancelled = Canceled


## Formats

# 12-hour clock, month before day.
bar-clock-format = %A, %B %-d  %-I:%M %p

## Top bar — battery

# The battery indicator's menu. $percent is a whole number.
bar-battery-percent = Battery { $percent }%
# $time is a duration written as hours:minutes, e.g. 2:14.
bar-battery-remaining = Battery { $percent }% — { $time } remaining
bar-battery-charging-time = Battery { $percent }% — { $time } until full
bar-battery-full = Battery { $percent }% — fully charged
bar-battery-charging = Battery { $percent }% — charging
# On the charger, but held at a charge limit or waiting to start.
bar-battery-plugged = Battery { $percent }% — plugged in, not charging
# $avg and $max are frequencies in GHz, already rounded, e.g. 2.80.
bar-cpu-frequency = CPU { $avg } GHz average, { $max } GHz peak
# $governor is the kernel's own name for the policy, e.g. powersave.
bar-cpu-governor = Governor: { $governor }
# The power-profiles-daemon profiles, which are the same three everywhere.
bar-power-saver = Power Saver
bar-power-balanced = Balanced
bar-power-performance = Performance
# Last entry in the battery menu.
bar-power-settings = Power Settings…
# What a screen reader calls the battery indicator.
a11y-battery = Battery

# Month before day, and a comma after it.
files-date-modified = { $month } { $day }, { $year } at { $time }
