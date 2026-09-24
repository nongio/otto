# Dictation

**Status:** draft
**Related specs:** [launcher.md](./launcher.md), [localisation.md](./localisation.md)

## Summary

Speech typed into a text field. While the user speaks, the words heard so far
appear at the caret and settle into the field as the recogniser becomes sure of
them. Recognition runs on a local speech server, so audio never leaves the
machine. An otto-kit app opts in for its own fields; a standalone input method
offers the same in any field that supports input methods.

## Goals

- Words appear at the caret within about a second of being said, and settled
  words never change once typed.
- The field shows plainly that it is listening, and that it hears the user.
- Stopping loses nothing: the last words said before the stop are typed in.
- Cancelling leaves the field as it was before dictation started.
- Names the field expects, such as the launcher's apps, come out spelled as
  those names.
- Recognition is local, and the engine can be swapped without touching the
  apps.

## Non-Goals

- Speech output (reading answers aloud).
- Cloud recognition services.
- Settings for dictation in Otto's configuration or the Settings app; for now
  it is set through the environment.
- Voice commands. What is said is text, never an instruction to the app.
- Managing the speech server. It is a user service the user installs and runs.

## Behavior

### Dictating into a text field

Dictation is a feature of otto-kit's text field that an app switches on. The
app decides which keys start, stop and cancel it; the field handles the rest.

**Starting.** When dictation starts, any selection in the field is removed, as
typing would, and the text is inserted at the caret from then on. The
microphone is the system's default source.

**Listening.** While listening:

- The caret is replaced by an equaliser: a row of five bars in the caret's
  colour, as tall as the line, one per band of the speech range from voice
  pitch up to sibilants. Each bar rises quickly with sound in its band and
  falls back more slowly.
- Words heard but not yet settled are drawn dimmed, in the placeholder's
  colour, at the caret and before the equaliser. They are not part of the
  field's value.
- Text after the caret is pushed along to make room, and the field scrolls so
  the equaliser stays in view.
- The placeholder and any completion hint are hidden, even while the field is
  still empty.

**Settling.** Twice a second, once at least half a second has been heard, what
was heard is sent for recognition. A word is settled when two passes in a row
agree on it, ignoring case and punctuation. Settled words are typed into the
field at the caret, exactly as if typed on the keyboard, and are never taken
back by later passes. A space is put in front of them when the character before
the caret is not a space, unless they start with punctuation. Sounds the
engine marks as non-speech, such as `[BLANK_AUDIO]` or `(music)`, are dropped.

**The noise floor.** Each bar measures its band against the room's background
level, not against silence. The background level drops at once to anything
quieter and creeps up towards a steady sound at about 10 dB a second, so within
a few seconds a humming fan or a noisy room rests the bars at their minimum
height, while speech, which comes and goes, stays above it and moves them. A
sound starts to show 6 dB above the background and fills a bar at about 46 dB
above it. Digital silence at the start of a capture does not pull the floor
down so far that the bars jump to full height when the room's sound arrives.

**Stopping.** When dictation stops, the microphone closes and everything heard
is recognised once more. The equaliser rests at its minimum height while that
final pass runs. When it returns, the words not yet settled are typed in, the
dimmed words and the equaliser disappear, and the ordinary caret comes back.

**Cancelling.** When dictation is cancelled, the microphone closes and every
character the dictation typed is removed. The rest of the field is left as it
was, with the caret where dictation started. The selection removed at the start
is not restored.

**Long dictation.** A dictation can run for as long as the user speaks. Passes
do not grow slower as it goes on.

### Dictation in the launcher

The launcher is the first app with dictation.

| Key | While not dictating | While dictating |
|---|---|---|
| Ctrl+D | Starts dictating into the query field | Stops |
| Escape, Backspace | As usual | Cancels, removing what was dictated |
| Return, keypad Enter | As usual | Stops, then acts once the last words are in |
| Ctrl, Alt, Super, Meta, Caps Lock alone | As usual | Ignored |
| Any other key | As usual | Stops; the key does nothing else |

- While dictating, keys are taken by dictation and are not also typed or
  acted on.
- After Return, the launcher does what Return does with the field as it then
  reads (launch the selected item, or send the request in ask mode), once the
  final pass has typed the last words in. Cancelling before then also cancels
  the send.
- The list refilters whenever dictation types into the field, just as it does
  for typing.
- When the launcher loses the keyboard, dictation stops as if a key had been
  pressed. The launcher normally closes on losing the keyboard as well.

### Vocabulary

When the launcher is picking from a list, the titles of the items listed when
dictation starts are its vocabulary. In ask mode the request is free speech and
there is no vocabulary.

- The vocabulary is sent with each pass, as a prompt for engines that take one
  and as hotwords for engines that favour given words while decoding. At most
  100 names are sent as hotwords, and the prompt is kept to about 200
  characters of names followed by the end of what has been settled so far.
- A run of up to four heard words that comes close to a name is written as that
  name, in both the dimmed and the settled words: "ghost tea" becomes
  "Ghostty", "fire fox" becomes "Firefox". Leading and trailing punctuation is
  kept.
- How close is close depends on the name's length: names of five or six
  letters and digits allow one edit, longer ones two.
- Names shorter than four letters and digits, or longer than four words, are
  not in the vocabulary, since short names match too much ordinary speech.
- A word that already spells a name is left alone, case and all: "settings"
  stays "settings" even when "Settings" is listed.

Without a vocabulary, only the end of what has been settled is sent as the
prompt, so a pass that starts mid-sentence keeps its context.

### Language

- The language is taken from the system locale (`LANG`): `it_IT.UTF-8` means
  Italian. When `LANG` is unset or `C`, the engine detects the language.
- `OTTO_DICTATE_LANGUAGE` overrides it with an ISO 639-1 code, or `auto` to
  let the engine detect it.
- The language is sent with every pass. Parakeet detects the language itself
  and ignores it.

### Engines

Recognition runs on a speech server at `http://127.0.0.1:8080/inference`,
speaking whisper.cpp's server API. `OTTO_DICTATE_URL` points elsewhere. Three
engines can serve it, each as a user service; one runs at a time, and starting
one stops the others.

| Engine | Languages | Notes |
|---|---|---|
| Parakeet (the default) | 25 European, detected | Ignores the language, prompt and hotwords |
| Whisper | English | Uses the prompt |
| CrispASR, serving Parakeet | 25 European, detected | Uses the hotwords |

`OTTO_DICTATE_HOTWORDS_BOOST` sets how strongly hotwords are favoured (default
4).

**When the engine is unreachable.** A pass that cannot reach the server, gets
an error back, or gets an answer with no timed words counts as a pass that
heard nothing. Nothing is shown to the user:

- While listening, the equaliser still moves, but no words appear, dimmed or
  settled.
- On stopping, the final pass fails the same way, dictation ends and the field
  keeps whatever had already been typed.
- A pass that gets no answer is given up after 20 seconds. No new pass is sent
  while one is outstanding, so with a server that hangs, words come at most
  once every 20 seconds and stopping can take that long to finish.
- In the launcher, Return still acts once the final pass is given up, on the
  field as it then reads.

When the microphone cannot be opened, the bars stay at rest, no pass is sent,
and stopping ends dictation with nothing typed.

### The otto-dictate input method

otto-dictate brings dictation to any field whose app supports input methods.
It runs for the whole session, started from autostart, as the seat's input
method.

- `otto-dictate toggle` starts dictating into the focused text field, and
  stops it again. No key is bound to it by default; the user binds a shortcut
  in Otto to that command.
- With no text field focused, toggle does nothing.
- While listening, a balloon under the caret shows an equaliser and the words
  heard so far, settled words bright and the rest dimmed. It grows to three
  lines, then shows the latest.
- The field is not touched while listening. When dictation stops, the whole
  text goes into the field at once, with a space in front when the character
  before the caret is not a space.
- While listening, the keyboard belongs to otto-dictate and no key reaches the
  app:
  - Escape discards everything; the field is left untouched.
  - Backspace drops the last settled phrase and what is still pending, and
    keeps listening.
  - Return, keypad Enter or any other key stops, like the toggle. The key is
    not passed on.
  - Modifier and lock keys are ignored, so the shortcut that toggled it does
    not stop it.
- When the field loses focus while listening, the dictation is dropped and
  nothing is typed.
- When another input method already holds the seat, otto-dictate asks again
  every three seconds, so it takes over once the other one goes.
- It has no vocabulary: no names are sent and nothing is corrected. Language
  and engine are chosen as above.

## Constraints & Edge Cases

- Recognition never blocks the app. A slow or hung engine delays words, never
  typing, drawing or input.
- A pass that returns after a later buffer has replaced its audio is dropped,
  so stopping never types words from a stale pass.
- A word the engine repeats across a cut in the audio is not typed twice.
- The dimmed words and the equaliser take room at the caret without becoming
  part of the value: the value, the caret offset and copy and paste see only
  settled text.
- In the launcher, Return with nothing heard acts on the field unchanged, which
  with an empty query means the first listed item.

## Rationale

- **Settle on agreement.** A single pass over half-said words is often wrong
  at its end. Typing only what two passes agree on keeps the field free of
  words that would have to be taken back, while the dimmed words show that
  speech is being heard.
- **The equaliser stands in for the caret.** It says where the words will go
  and that the microphone hears the user, without a separate indicator.
- **A noise floor, not a fixed scale.** Against a fixed scale, a noisy room
  keeps the bars up the whole time and they stop saying anything about the
  voice. Measuring against the room's own background means only speech moves
  them.
- **Correcting to the vocabulary after recognition.** Only one engine takes
  hotwords and only one takes a prompt. Correcting heard words to near names
  helps every engine, and hotwords or a prompt help where they are understood.
- **No vocabulary in ask mode.** A request to an agent is free text; forcing
  it towards app names would corrupt ordinary words.
- **Cancelling removes what was dictated.** Escape and Backspace are the keys
  that already mean "undo that"; the user should not have to delete dictated
  text by hand.
- **Any other key stops.** Starting to type means the user has finished
  speaking. Modifiers are exempt because the start shortcut uses one.
- **A local server on a fixed port.** Apps talk to one address and the user
  picks the engine by starting its service, without reconfiguring anything.
- **otto-dictate commits once.** Fields in other apps may not handle text that
  arrives piece by piece and is later corrected, so the text is committed only
  when the user stops.
- **otto-dictate retries.** The seat takes one input method. Retrying lets it
  take over after another one exits, without the user restarting it.

## Open Questions

- Should an unreachable engine be reported to the user, for example in the
  placeholder, rather than only in the logs?
- Should dictation be configured in Otto's settings, with the language taken
  from Otto's own locale setting rather than `LANG`?
- Should otto-dictate have a default shortcut, and should the standalone
  input method and in-app dictation share one?
- Should Backspace in the launcher drop the last phrase, as otto-dictate does,
  rather than cancel the whole dictation?
