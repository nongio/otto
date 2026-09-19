# Audio

Otto handles two audio concerns: **volume control** from the media keys, and
**UI sound effects** for feedback.

## Not an audio server

It is not an audio server — PipeWire or PulseAudio does the actual mixing. Otto
talks to whichever you run.

## Volume keys

```toml
[keyboard_shortcuts]
"XF86AudioRaiseVolume" = "VolumeUp"
"XF86AudioLowerVolume" = "VolumeDown"
"XF86AudioMute"        = "VolumeMute"
```

Volume moves in 5% steps. Each change shows an on-screen indicator and, if sound
feedback is on, plays a short click so you can hear where the level is.

## Media keys

```toml
"XF86AudioPlay" = "MediaPlayPause"
"XF86AudioNext" = "MediaNext"
"XF86AudioPrev" = "MediaPrev"
"XF86AudioStop" = "MediaStop"
```

These control whichever media player is registered on MPRIS — Spotify, VLC,
mpv with the MPRIS script, browsers playing video. No configuration needed;
Otto talks to the active player.

## Sound effects

```toml
[audio]
sound_enabled = true
sound_theme = "freedesktop"
```

`sound_enabled = false` turns off all UI feedback sounds.

### Sound themes

`sound_theme` names an XDG sound theme, following the
[freedesktop Sound Theme specification](https://specifications.freedesktop.org/sound-theme-spec/latest/).
Sounds are looked up at:

```
/usr/share/sounds/{theme}/stereo/{event}.oga
```

Commonly available themes: `freedesktop`, `Pop`, `ocean`. See what you have:

```sh
ls /usr/share/sounds/
```

Comment `sound_theme` out and Otto auto-detects an installed theme.

### Custom sounds

Drop your own files in Otto's `resources/` directory to override the theme:

```
resources/audio-volume-change.oga
```

Custom files take precedence over the theme's version of the same event.

## Choosing an output device

Otto has no audio device picker. Use your audio server's tools:

```sh
pavucontrol          # graphical, works with PipeWire and PulseAudio
wpctl status         # list PipeWire devices
wpctl set-default 42 # make node 42 the default sink
```

`pavucontrol` in the system tray is the usual setup — see
[Top Bar](topbar.md).

## Audio over remote sessions

Neither [screen sharing](screen-sharing.md) nor the
[RDP bridge](remote-desktop.md) carries audio. They are video-only. Route audio
separately if you need it — for instance with a PipeWire network sink.

## Troubleshooting

**Volume keys do nothing.** Check they are bound: not every keyboard produces
`XF86AudioRaiseVolume`. Run `wev` or `./scripts/show-keys.sh` and press the key
to see what it actually emits, then bind that keysym.

**Volume changes but nothing gets louder.** Otto is adjusting a different sink
from the one playing. Check `wpctl status` for which sink is default.

**No feedback sound.** Confirm `sound_enabled = true`, that the theme is
installed (`ls /usr/share/sounds/`), and that the file for the event exists in
the theme's `stereo/` directory. Many themes are incomplete.

**Media keys do nothing.** The player must expose MPRIS. Check with:

```sh
busctl --user list | grep mpris
```

Some players need a plugin (mpv) or a setting (Spotify's `--enable-mpris`).
