# Localisation

**Status:** draft — implemented across the workspace, catalogues still growing
**Related specs:** [settings-app.md](./settings-app.md), [topbar.md](./topbar.md),
[quickview.md](./quickview.md), [login-mode.md](./login-mode.md),
[lock-screen.md](./lock-screen.md), [launcher.md](./launcher.md),
[file-browser.md](./file-browser.md), [file-picker.md](./file-picker.md)

## Summary

Every string Otto itself writes — the compositor's menus, the bar, the
launcher, Files, Settings, Quick View, the greeter and the lock screen — is
looked up by a stable key in a per-locale catalogue rather than written into
the code. One language is chosen once, at process start, from Otto's own
preference rather than from the shell environment, and every part of the
desktop agrees on it.

## Goals

- A user who sets a language in Settings gets that language in every part of
  Otto's interface, whatever `LANG` the session was started with.
- Adding a string, or adding a language, needs no change to any component.
- A missing translation degrades to English, never to a crash and never to a
  blank label.
- The same catalogue serves a component that has a session bus and one that
  has been deliberately cut off from it.
- A locale that is not fully translated is still usable: untranslated keys fall
  through to English rather than leaving the interface half-rendered.
- The gaps are found by tests, not by users reading a pane in their own
  language and finding half of it in English.

## Non-Goals

- Translating text Otto does not author. PAM and greetd conversation text, the
  operating system's error strings, application-supplied notification and
  dialog text, and the names applications give themselves are passed through as
  they arrive.
- Changing language without restarting. See *Rationale*.
- Right-to-left layout. No RTL locale ships, and the mirroring the top bar
  describes is unimplemented.
- Locale-aware collation. Names sort by a natural-order comparison with ASCII
  case folding; see [file-picker.md](./file-picker.md).
- Region formatting as a separate axis. One preference chooses both the
  language and the date, time and number conventions that go with it.
- Translating configuration files, identifiers, or anything on the wire.

## Behavior

### Catalogues

Strings live in one Fluent catalogue per locale, keyed by stable identifiers
rather than by their English text. Ten catalogues ship: `en-GB`, a sparse
`en-US`, and full translations for `de`, `es`, `fr`, `it`, `pl`, `pt-BR`, `ru`
and `uk`. They are compiled into the binaries, so a component can render its
first frame before any filesystem the user controls is necessarily mounted, and
a catalogue on disk cannot drift out of step with the keys the code asks for.

`en-GB` is the source of truth and the only catalogue guaranteed to carry every
key — 448 of them at the time of writing. `en-US` is an overlay: it carries
only the keys whose spelling or format actually differs (colour/color, the
12-hour clock, month-before-day dates) and everything else falls through.

A key is looked up through a chain of locales, most specific first, always
ending at `en-GB`. A requested tag contributes its own candidates before its
fallbacks: `pt_BR.UTF-8` becomes `pt-BR` then `pt`; `de_DE@euro` becomes `de-DE`
then `de`. POSIX spellings are accepted alongside BCP 47. `C` and `POSIX`
contribute nothing. A bare `en` resolves to `en-US`, because that is what a bare
`en` means nearly everywhere it appears, even though Otto authors in British
English. Candidates with no catalogue are skipped rather than treated as an
error.

A key no bundle in the chain carries is a bug, not a runtime failure: the key
itself is rendered, so the gap is visible in the interface — `dock-keep-in-dock`
where a label should be — and logged, rather than taking the desktop down.

### Where the language comes from

In priority order:

1. **The compositor's `locales` setting**, a list of tags, most preferred first.
   This is what the *Preferred languages* row in Settings writes.
2. **The environment**, consulted only when the setting cannot be read or is
   empty: `LC_ALL`, `LC_MESSAGES`, `LANG`, `LANGUAGE`, in that order, with
   `LANGUAGE` read as a colon-separated priority list and the others as single
   tags.

The compositor reads its own setting directly. Every other Otto component reads
it from the compositor over the desktop portal, under `org.otto.desktop`
`locales` — the same door the colour scheme and accent settings come through —
which the portal backend answers by asking the compositor's settings service.
A component that cannot reach the portal within two seconds falls back to the
environment, which covers a component started before the portal is up, a
component run outside an Otto session, and the tests.

Otto's own setting outranks the environment deliberately. `LANG` describes the
session the user was logged into; the setting describes what the user asked
for, in Otto, after the fact. A user who picks Italian in Settings while the
session was started with `LANG=en_GB` must get an Italian desktop — not an
Italian compositor bolted to English components, which is what deferring to the
environment per-process would produce.

The setting's shipped default is `en`. A user on an Italian system therefore
gets an English desktop until they change the setting; leaving the list empty
is what asks Otto to take the language from the environment instead.

Language is resolved once, before the first string is looked up and before
anything is drawn. The first resolution wins for the life of the process.

### Not hot-reloadable

`locales` is a restart-required setting. Changing it is validated, persisted
and announced like any other, and takes effect at the next start of each
process; a running bar, or a running Settings window, keeps the language it
started in. There is no watcher for the portal key.

This is a property of how strings are handed out, not an oversight: a lookup
yields a reference that lives as long as the process, which is what lets a
translated string drop into the fixed labels Otto's menus and settings rows are
built from. Nothing can invalidate those references, so nothing can change the
language under them.

### Text that is not translated

Otto translates what Otto writes, and only that:

- **PAM and greetd conversation text.** Prompts, info and error messages
  arriving from the authentication stack are displayed as they arrive. Those
  stacks localise themselves, and restating them would be guessing at another
  program's words. The greeter's and lock screen's *own* labels — the username
  and password prompts it shows before greetd asks anything, its status lines,
  the button that abandons the fingerprint reader — are translated.
- **Operating-system error text.** Where a message interpolates an error, only
  the sentence around it is translated; the error itself arrives in whatever
  language the system produced it, usually English.
- **Application-supplied text.** Notification summaries and bodies, portal
  dialog text supplied by the requesting app, and window titles.
- **Font, theme and device names.** Font families come from fontconfig, icon,
  cursor and GTK theme names from the themes themselves, and greeter and locker
  names from what is installed. These are proper names of things on the machine,
  and a translated one would name nothing.
- **Identifiers on the wire.** Setting identifiers, D-Bus interface, service and
  member names, configuration keys, and the modifier names in a shortcut
  (`Ctrl`, `Alt`, `Super`) are literal syntax.

### Application names

The name an application shows in the dock, the launcher and the app switcher is
the application's own, not Otto's — but it is read in Otto's language. A desktop
entry that carries a `Name[xx]` for the chosen locale is read under that key,
falling back through the POSIX form (`Name[pt_BR]`), the bare language
(`Name[pt]`) and finally the plain `Name=`. The locale asked for is the
interface locale resolved above, not `LANG`: an app's label under the pointer
has to be in the same language as the menu that opens over it.

Otto's own entries carry no `Name[xx]` yet, so they read as English until they
do.

### The language applications see

Otto's own components are told the language over the portal, but an ordinary
application is not: GTK, Qt and gettext all read the environment, and a session
started with `LANG=en_US.UTF-8` but set to Italian would draw Italian chrome
around English applications.

So the compositor publishes the configured language into the environment as it
starts, before anything is spawned:

- `LANGUAGE`, gettext's colon-separated priority list, always. Each configured
  tag contributes its POSIX form and its bare language (`pt_BR:pt`), and the
  list is honoured whether or not those locales were ever generated on the
  machine.
- `LANG` and `LC_MESSAGES`, only when the locale actually exists here — named
  the way `setlocale` wants it, `it_IT.UTF-8`. Naming a locale that was never
  generated is worse than saying nothing: `setlocale` fails and the application
  falls back to C, losing the very translation this was meant to supply. A tag
  with no region (`it`) is matched against what is generated to find one.
- `LC_ALL` is never written. It overrides everything, so a user or a session
  script that set it meant to.

These go into the compositor's own environment, which every application it
launches inherits, and — alongside `WAYLAND_DISPLAY` — into the systemd and
D-Bus activation environments, since a bus-activated service is not Otto's
child and inherits nothing from it.

This is what makes an application's *own* strings follow the setting: its
menus, which reach the bar over DBusMenu already translated by the application,
and the `Name[xx]` in its desktop entry.

### Format keys

Three keys are strftime patterns rather than prose, and each locale rewrites
them to its own convention rather than translating them:

- `bar-clock-format` — the top bar's clock. 24-hour and day-before-month in
  most locales; `en-US` overrides it to a 12-hour clock with the month first.
  An explicit `clock_format` in the bar's own configuration still wins over the
  catalogue. Whether seconds are shown is a user setting and changes how often
  the bar redraws, so a translation must not add or remove `%S`.
- `auth-clock-time-format` and `auth-clock-date-format` — the large clock above
  the login and lock cards. Only the `%`-codes and the separators between them
  belong to the translator; the weekday and month names inside them are
  rendered by the date library against the locale, and must not be spelled out
  in the pattern.

Rendering those names in the user's language needs a locale in POSIX form
(`pt_BR`, `ru_RU`), which the resolved language tag is turned into: a tag that
already carries a region keeps it, and a bare language subtag is paired with
the conventional default region for that language. A tag the date library does
not know still produces a clock, in the source locale's names.

### Settings labels

The compositor serves each setting's label and description, and both are
translated on the way out. The catalogue keys are derived from the setting's
own identifier rather than written by hand — `dock.autohide` becomes
`schema-dock-autohide-label` and `schema-dock-autohide-description` — so adding
a setting needs no catalogue entry to keep working: the English written beside
it in the schema is the fallback, and a new setting is untranslated rather than
broken. Enumerated choices whose labels are catalogue keys — the colour scheme,
the named accent colours — are resolved the same way; a choice label that is not
a key, such as a discovered font or theme name, passes through untouched.

### Components with no bus

A component that has deliberately given up its session bus cannot ask the
portal. Quick View's decode worker is the case that exists: it is re-executed
with a cleared environment and no Wayland or D-Bus socket, because those are
capabilities, and it writes strings a person reads — why a preview is
unavailable, the facts on a card. Its parent therefore forwards the language it
resolved in `LANGUAGE`, and the worker resolves from the environment. A locale
tag is not a capability, and without the hand-off the worker's strings would be
English while the rest of the desktop was not — silently, since nothing else
about the preview would look wrong.

Any future sandboxed helper must do the same: forward the parent's resolved
locale and resolve from the environment in the child.

### Adding a string

1. Add the key to `resources/locales/en-GB.ftl`, in the section for the
   component that shows it, with a comment saying where it appears and what
   constrains it — the width it must fit, whether it wraps, what each variable
   holds.
2. Use it from the code by key. Prefer the interned form for a label that is
   shown repeatedly; use the owned form for a string built from unbounded input
   such as a file name or a window title, where interning would grow without
   limit.
3. Add the key to every other catalogue, or the parity test fails. The
   `l10n-translator` agent in `.claude/agents/` translates in Otto's voice and
   knows the conventions the catalogues follow.
4. A new setting also needs its two `schema-*` keys, or the compositor's
   coverage test fails.

A value the code has to *recognise* carries the key, not the translated text.
`SaveAction::Blocked` names the reason a save is refused, and the picker treats
one of those reasons differently from the rest — an empty name field is the
placeholder's job to explain, not the warning line's. Carrying the message
instead made that comparison hold only in English, so every other locale grew a
warning under a field the user had not typed in yet. The lookup belongs at the
point the string is drawn.

### Adding a locale

1. Copy `en-GB.ftl` to `resources/locales/<tag>.ftl` and translate every key,
   keeping every variable each English string uses.
2. Rewrite the three format keys to the locale's convention rather than
   translating them.
3. Register the catalogue so it is compiled in.
4. If the language's conventional region is not already known to the POSIX
   mapping, add it, or the clock's weekday and month names stay in English.

A regional variant of a language that already ships can instead be added as a
sparse overlay, carrying only the keys that differ, the way `en-US` overlays
`en-GB`.

### Tests

- Every catalogue parses, and the source catalogue parses.
- Every translated catalogue carries every key `en-GB` does. `en-US` is exempt:
  it is a sparse overlay by design.
- No catalogue invents a key `en-GB` does not have — a typo that would otherwise
  be silently unreadable forever.
- No translation drops a variable its English string uses. A count that never
  reaches the text reads as a missing number.
- The locale chain resolves POSIX spellings, bare `en`, and the Chinese scripts
  as described above.
- The portal read gives the same answer whether or not a runtime is already
  running, exercised from a multi-threaded runtime, from a current-thread
  runtime, and from no runtime at all: in `portal.rs`,
  `reads_from_inside_a_multi_thread_runtime`,
  `reads_from_inside_a_current_thread_runtime` and `reads_with_no_runtime_at_all`,
  which all assert through `agrees_with_an_off_runtime_read`.
- In the compositor, `every_setting_has_catalogue_entries` — every setting has
  both of its strings in the catalogue — and `every_schema_key_names_a_real_setting`
  — no `schema-*` key names a setting that no longer exists. Neither is a
  correctness requirement, since both directions fall back safely; they exist
  because both failures are invisible in a running desktop.
- A setting the catalogue has never heard of still renders the English beside it
  in the schema.

No test asserts a user-facing string as English prose. The chain a test binary
resolves comes from the environment, so a developer whose own session is Italian
runs the whole suite against the Italian catalogue — and a test spelling out
"0 bytes" or "Undid Move" then fails on a change that is entirely correct. A
test that needs the text compares against the same lookup the code performs,
which still pins what the test is actually about: which unit a size crossed into,
which operation the undo stack recorded, which month `civil_from_days` landed on.

## Constraints & Edge Cases

- **A runtime inside a runtime.** The portal read must not build a runtime on
  the thread that calls it. Otto's components do not agree on what `main` is:
  otto-bar, otto-islands and otto-quickview drive a Tokio runtime from `main`
  itself, while otto-settings, otto-files, otto-launcher, otto-greeter and
  otto-lock are synchronous. Building a runtime on a thread already inside one
  panics outright — it does not fail — so a read done on the caller's thread
  works in five components and kills three at startup. It is what stopped
  otto-bar launching at all. The read therefore happens on a thread of its own,
  which belongs to no runtime, and is correct from either kind of `main`.
- **The read is blocking, and must be.** It runs before anything is drawn; a
  component cannot render its interface until it knows what language it is in,
  so there is nothing useful to do concurrently. A missing or wedged portal must
  not hold up startup, hence the two-second bound.
- **The greeter has no session.** It runs before any user session exists, so
  the portal is usually absent and the greeter resolves from its own
  environment. A login screen in a language other than the environment's needs
  the greeter's environment set, not the setting.
- **Resolution is once per process.** A component that looks up a string before
  resolving the language fixes the chain from the environment for the rest of
  its life. Language must be resolved as the first thing `main` does, before any
  chrome is built — the compositor's dock menus, for instance, are assembled
  during construction.
- **Fluent's bidirectional isolation is off.** The isolating marks Fluent wraps
  around interpolated values render as stray boxes rather than as nothing, and
  no RTL locale ships. A locale that needs them would need this reconsidered.
- **A catalogue that fails to parse** is skipped with an error rather than
  aborting: the chain falls through to the next locale, and ultimately to
  `en-GB`.
- **Plurals belong to the catalogue.** A count is passed as a variable and the
  catalogue selects the form; a language with more than two plural categories
  is expressible without touching the code.
- **Byte units are SI.** Otto counts in powers of 1000, so the abbreviations are
  KB, MB, GB, TB. Most languages keep the symbols and translate only the
  spelled-out "bytes".

## Rationale

- **Keys, not English text, as the lookup.** Editing an English string would
  otherwise silently orphan every translation of it. It also lets the same
  English word carry different translations in different contexts, which several
  of the menu and settings choices need.
- **Catalogues compiled in rather than read from disk.** A desktop that cannot
  find its own strings is not a recoverable state, and the compositor draws
  before the user's filesystem is necessarily available. Compiling them in also
  means a stale file cannot disagree with the keys the binary asks for.
- **Strings live for the life of the process.** This is what lets a lookup drop
  into the fixed labels Otto's menus and settings rows already use, without
  reworking every widget to own its text. The cost is that language cannot
  hot-reload, which is the trade taken: a language change is rare and a restart
  is understood, whereas ownership churn would be paid on every string, forever.
- **`en-GB` as the source.** Otto's English is authored in British English, and
  one catalogue has to be the complete one. `en-US` as a sparse overlay rather
  than a full copy keeps the two English variants from drifting: a duplicated
  unchanged string would stop tracking later edits to the source.
- **A missing key renders as the key.** Loud enough to be found in review,
  quiet enough not to take the desktop down. The alternative — rendering nothing
  — hides the bug behind a plausible-looking blank label.
- **Settings labels keyed off the setting identifier.** The schema is built at
  compile time and cannot hold lookups; deriving the keys means adding a setting
  cannot forget to add a string, only to translate one, and the English in the
  schema stays both the fallback and the thing translators translate.
- **The setting outranks `LANG`.** The setting is what the user actually
  changed. Reversing this would make the Settings row decorative on any session
  whose environment names a language.

## Open Questions

- The chain resolves `zh` and `zh-CN` to `zh-Hans`, and the POSIX mapping knows
  `ja`, but no Chinese or Japanese catalogue ships. Either the catalogues follow
  or the mappings are speculative.
- An empty `locales` list means "use the environment" for components reading it
  over the portal, but the compositor resolving its own empty list goes straight
  to `en-GB` instead. The two should agree.
- The settings pane labels the row *Preferred languages* while the schema calls
  the setting *Locales*. One name should win.
- Whether the language preference should also drive `LANG` for applications the
  user launches, so a translated Otto does not sit around untranslated apps.
- Whether a region preference separate from the language is worth having — a
  user who wants an English interface with European dates has no way to say so
  short of an overlay catalogue.
- Where a translation that no longer fits its widget is caught. The catalogues
  document the space each string has, but nothing measures it.
