#!/usr/bin/env python3
"""Prints one field of the current month for the calendar widget.

Usage: month.py KEY   (month, num, year, days, weekends, begins, ends, issue,
w1 to w6). Without a key it prints every field as JSON.
"""
import calendar
import datetime
import json
import sys

today = datetime.date.today()
year, month = today.year, today.month
ndays = calendar.monthrange(year, month)[1]
first = datetime.date(year, month, 1)
last = datetime.date(year, month, ndays)
saturdays = sum(
    1 for d in range(1, ndays + 1) if datetime.date(year, month, d).weekday() == 5
)

lines, line = [], []
for d in range(1, ndays + 1):
    date = datetime.date(year, month, d)
    entry = f"{d:02d}, {date.strftime('%A')}/"
    line.append(f"<b>{entry}</b>" if date == today else entry)
    if date.weekday() == 6:
        lines.append(" ".join(line))
        line = []
if line:
    lines.append(" ".join(line))

fields = {
    "month": first.strftime("%B"),
    "num": f"{today.day:02d}",
    "year": str(year),
    "days": f"{ndays} Days",
    "weekends": f"{saturdays} Weekends",
    "begins": f"Begins {first.strftime('%A')}",
    "ends": f"Ends {last.strftime('%A')}",
    "issue": f"Week {today.isocalendar().week:02d}",
    # Poll keeps one line of output, so each week gets its own key.
    # Weeks fill the last slots so the final line always sits on the rule.
    **{f"w{i + 1}": w for i, w in enumerate([""] * (6 - len(lines)) + lines)},
}

if len(sys.argv) > 1:
    print(fields.get(sys.argv[1], ""))
else:
    print(json.dumps(fields))
