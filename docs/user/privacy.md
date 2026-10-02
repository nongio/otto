# Privacy

What apps and agents are allowed to do on your desktop, how you can tell,
how to take an answer back, and when Otto asks for your password.

## Agents on your desktop

A program can ask Otto to let an AI agent work beside you: a cursor of its
own, in a colour of its own, on a workspace of its own. The agent clicks and
types there while you carry on elsewhere. Your cursor, your keyboard and the
workspace you're looking at stay yours.

### Being asked

- **The first time a program asks**, Otto asks you: *Let <app> use an
  agent cursor?* Allow it once and it isn't asked again; you can change
  your mind in Settings › Privacy (below).
- **A workspace of its own** needs nothing more. Otto adds one after your
  last workspace, named after the agent, and doesn't switch you to it.
- **One of your workspaces** is asked about every time: *Let <agent> work
  on <workspace>?* While it's lent, the agent sees the windows there and
  can click and type in them, beside you.

The dialog ignores a click or Enter in its first moment on screen, so a
keystroke you were already making can't answer it.

### Seeing it

- A workspace an agent can act on has a **frame in the agent's colour**
  around the screen, with a chip at the top showing its name and **Stop**.
  Over a fullscreen window the chip hides and the frame stays.
- In exposé and the workspace switcher, its thumbnail has a ring in the
  agent's colour and a small cursor badge.
- The agent's **cursor** carries its name and is drawn only on its
  workspace. It fades when the agent pauses and comes back when it moves.
- Otto never brings an agent's workspace to you. Go there when you want to
  watch: you can click and type in the agent's windows too.

### Stopping it

- **Stop** on the chip ends that agent at once. Its program can't get
  another agent cursor until you log in again.
- **Ctrl+Alt+Shift+Esc** stops every agent at once. It comes from the
  system, below Otto, so no program can send it or block it. It works when
  your system's logind is set up to deliver it.
- When an agent stops, or finishes and leaves, its workspace and its
  windows stay, now plainly yours: the frame goes, a workspace made for the
  agent loses the agent's name, and the windows it opened keep working for
  you.

### What an agent can't do

- Move your cursor, take your keyboard, bring a window in front of you or
  switch your workspace.
- Reach any workspace but the one it was given, or Otto's bar, dock and
  dialogs.
- Capture anything but its own workspace, or anything while the screen is
  locked.
- Act at all while the screen is locked.

GTK apps you already had open when you lend a workspace (gedit, GNOME
apps) don't notice the agent's cursor, so it can see them but can't type
in them. Apps you open during the loan, and Qt apps, work.

## Settings › Privacy

**Agents** lists every program that asked for an agent cursor, each with a
switch and **Forget**. Turning one off stops its agents straight away;
forgetting it means it's asked again next time.

**Screen sharing** lists:

- **Remembered screen shares.** An app that shared your screen and was
  remembered can share again without the picker. Each row says what it
  shares ("Records the screen eDP-1", "Records a window"). **Forget** removes
  it, and the app has to ask again next time.
- **Remote desktop** shares that were remembered, the same way.
- **Screenshots.** Apps that asked to take screenshots without the
  screenshot dialog, each set to **Ask**, **Allow** or **Don't Allow**.
  **Forget** puts it back to asking.

**Notifications** lists the apps that sent a notification through the
notification portal (Flatpak apps), each with a switch. Turning one off stops
its notifications. **Forget** drops your answer, so the app is asked again the
next time it notifies.

The page reads xdg-permission-store, where these answers are kept, and
updates as soon as an answer changes anywhere: in the share picker, in
another desktop's settings, or with `flatpak permission-reset`. Apps outside
a sandbox can't be told apart by the portal, so they share one row,
labelled **Apps outside a sandbox**.

## Remembering a screen share

The screen-sharing picker has a **Remember for <app>** checkbox. Tick it and
the next time that app asks to share, Otto shares the same screen or window
without asking, as long as it is still there. It then appears under
**Screen sharing** in Privacy, where **Forget** undoes it.

The checkbox appears only for apps the portal can name: Flatpak apps, and
apps that bring their own app id. Apps started outside a sandbox all look the
same to the portal, and remembering for one would remember for all of them, so
they are always asked.

## Strict mode

By default, the programs you run yourself can do what programs on any
Linux desktop can: take screenshots, record the screen, type and click as
you, read the clipboard, and focus or close other windows. That's what
screenshot tools, clipboard managers and remote controls need.

If you'd rather only Otto's own programs could, turn on strict mode and
list the tools you trust:

```toml
# ~/.config/otto/config.toml
[privacy]
strict = true
allow = ["/usr/bin/grim", "/usr/bin/wl-paste"]
```

Apps then go through the portals, which ask you. Tools not on the list,
such as clipboard managers, `wlrctl` and other taskbars, stop working
until you add them. Restart Otto after changing this.

Sandboxed apps (Flatpak) never get any of this, strict or not.

## The password panel

Otto shows the same frosted card as the lock screen whenever something needs
your password:

- a program you run with `pkexec`, or a system service asking polkit (adding a
  printer, installing an update);
- a change to a setting that decides what runs in front of your password or
  whether the screen locks: the locker, the login screen program, auto-lock,
  locking on suspend, the lid and the power button.

The card says who is asking and what for. Type your password and press
Enter, or touch the fingerprint reader if your PAM setup uses one. **Cancel**
or Escape says no. While the card is up nothing else can take the keyboard
or be drawn over it.

Otto is the session's polkit agent unless you turn it off:

```toml
# ~/.config/otto/config.toml
polkit_agent = false
```

Do that only if you start another polkit agent yourself; without one,
`pkexec` can't ask for a password.

## What this doesn't cover

These answers keep **sandboxed apps** in check, and keep agents to the
workspace they were given. A program running as you, outside a sandbox,
can edit your configuration and the permission store directly, or type
below Otto if your system lets it write to `/dev/uinput`; Otto doesn't
pretend to stop that.

## See also

- [Screen Sharing](screen-sharing.md)
- [Lock Screen](lock-screen.md)
- [Ask and Agents](agents.md)
