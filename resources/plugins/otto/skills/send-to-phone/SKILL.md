---
name: send-to-phone
description: Send a file, picture or screenshot to the person's phone, into the chat app they talk to Otto from (Telegram and others, through the chat bridge). Use when they ask to send, share or show them a file, a picture or a screenshot while writing from their phone, or ask for something to be sent to their phone ("send me the screenshot", "share that PDF with me", "send this to my phone").
allowed-tools: Bash(cc-connect send *) Bash(grim *) Bash(mkdir -p *Pictures/Screenshots*) Bash(otto-msg -t get_outputs *)
---

# Send to the phone

Messages from a chat app reach you through the chat bridge, cc-connect. Your
text replies go back to the chat on their own; files and pictures do not. Send
them with:

```sh
cc-connect send --image /home/<you>/Pictures/Screenshots/files.png --message "The Files window"
cc-connect send --file /home/<you>/Documents/report.pdf
```

- **`--image`** for png, jpg, gif and webp; **`--file`** for anything else. Up
  to 50 MB. `--message` adds a caption.
- **Absolute paths under the home folder.** You run with a private `/tmp` that
  cc-connect cannot see, so save what you make (a screenshot, an export) under
  `~/Pictures/Screenshots` or your working folder, never `/tmp`.
- **Only say it was sent once the command says so.** `Message sent
  successfully.` means it is in the chat. `cc-connect is not running` means no
  chat bridge is on: say so, and say where the file is instead.
- **Send what was asked for.** A file goes to the person's own chat, but it
  still leaves the computer: don't send anything they did not ask for.

## Taking a screenshot

`grim` captures the screen. Otto cannot crop to one window yet, so bring the
window asked for to the front first, then capture its monitor:

```sh
otto-msg '[app_id="otto-files"] focus'      # the window asked for; see otto-help's desktop page for finding it
sleep 0.5                                   # let it come forward
mkdir -p ~/Pictures/Screenshots
grim ~/Pictures/Screenshots/files-$(date +%s).png            # every monitor
grim -o eDP-1 ~/Pictures/Screenshots/files-$(date +%s).png   # one monitor: names from `otto-msg -t get_outputs -r`
```

Then send it with `cc-connect send --image`. The capture is the whole screen,
so it can show more than the one window: other windows, notifications. Say so
when you send it.
