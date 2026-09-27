---
name: send-to-phone
description: Send a file, picture or screenshot to the person's phone, into the chat app they talk to Otto from (Telegram and others, through the chat bridge), and take the screenshot to send. Use when they ask to send, share or show them a file, a picture or a screenshot of the screen or of an app while writing from their phone, or ask for something to be sent to their phone ("send me a screenshot", "send me a screenshot of Files", "share that PDF with me", "send this to my phone").
allowed-tools: Bash(cc-connect send *) Bash(*/send-to-phone/scripts/screenshot *) Bash(*/send-to-phone/scripts/screenshot)
---

# Send to the phone

Messages from a chat app reach you through the chat bridge, cc-connect. Your
text replies go back to the chat on their own; files and pictures do not. Send
them with:

```sh
cc-connect send --image /home/<you>/Pictures/Screenshots/otto-files-20260927-191500.png --message "The Files window"
cc-connect send --file /home/<you>/Documents/report.pdf
```

- **`--image`** for png, jpg, gif and webp; **`--file`** for anything else. Up
  to 50 MB. `--message` adds a caption.
- **Absolute paths under the home folder.** You run with a private `/tmp` that
  cc-connect cannot see, so save what you make (a screenshot, an export) under
  `~/Pictures/Screenshots` or your working folder, never `/tmp`.
- **One command per call, as written here.** Don't chain them with `&&`,
  pipes or `sleep`: each command on its own is what the person has allowed
  once, and anything else asks them again, on their phone.
- **Only say it was sent once the command says so.** `Message sent
  successfully.` means it is in the chat. Otherwise, say where the file is and
  what went wrong:
  - `cc-connect is not running`: no chat bridge is on.
  - `no active session found`: the bridge is on, but this conversation is not
    coming from the chat right now: it was typed at the desk, or the bridge
    restarted since the last message from the phone. Their next message from
    the chat fixes it; don't say the bridge is off.
- **Send what was asked for.** A file goes to the person's own chat, but it
  still leaves the computer: don't send anything they did not ask for.

## Taking a screenshot

Use this skill's `scripts/screenshot`, by its full path under the skill's base
directory. It prints the path of the picture it took:

```sh
<skill dir>/scripts/screenshot                 # the whole screen, every monitor
<skill dir>/scripts/screenshot otto-files      # Files brought to the front first
<skill dir>/scripts/screenshot -o eDP-1        # one monitor
```

- **The app** is its app id: `otto-files`, `otto-settings`, `firefox`… The
  otto-help skill's desktop page says how to find one. `no window of … is open`
  means that app has no window: say so rather than capturing something else.
- **It is the whole screen.** Otto cannot crop to one window yet, so the
  picture can show more than was asked for: other windows, notifications. Say
  so when you send it.
- Don't use `grim` or a screenshot harness directly: the script is the one
  that is allowed without asking.

Then send the path it printed with `cc-connect send --image`.
