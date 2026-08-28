# Otto — American English
#
# A sparse overlay on en-GB. Only keys whose spelling or format actually
# differs belong here; everything else falls through the bundle chain to
# en-GB.ftl. Do not copy an unchanged string in — a duplicate silently stops
# tracking future edits to the source.


## Spelling

settings-accent-colour = Accent color
settings-background-colour = Background color


## Formats

# 12-hour clock, month before day.
bar-clock-format = %A, %B %-d  %-I:%M %p

# Month before day, and a comma after it.
files-date-modified = { $month } { $day }, { $year } at { $time }
