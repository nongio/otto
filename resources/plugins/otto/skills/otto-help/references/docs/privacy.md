# Privacy

What apps have been allowed to do, where to take an answer back, and when
Otto asks for your password.

## Settings › Privacy

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

The page reads xdg-permission-store, where xdg-desktop-portal keeps these
answers, and updates as soon as an answer changes anywhere: in the share
picker, in another desktop's settings, or with `flatpak permission-reset`.
Apps outside a sandbox can't be told apart by the portal, so they share one
row, labelled **Apps outside a sandbox**.

## Remembering a screen share

The screen-sharing picker has a **Remember for <app>** checkbox. Tick it and
the next time that app asks to share, Otto shares the same screen or window
without asking, as long as it is still there. It then appears under
**Screen sharing** in Privacy, where **Forget** undoes it.

The checkbox appears only for apps the portal can name: Flatpak apps, and
apps that bring their own app id. Apps started outside a sandbox all look the
same to the portal, and remembering for one would remember for all of them, so
they are always asked.

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

These answers keep **sandboxed apps** in check. A program running as you,
outside a sandbox, can edit your configuration and the permission store
directly; Otto doesn't pretend to stop that.

## See also

- [Screen Sharing](screen-sharing.md)
- [Lock Screen](lock-screen.md)
