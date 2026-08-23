# otto-kit Window Focus

**Status:** stable
**Related specs:** [window-decorations](window-decorations.md), [file-browser](file-browser.md), [settings-app](settings-app.md)

## Summary

otto-kit applications draw their own window decoration, so nothing outside them
marks which of their windows the user is working in. A kit window therefore
follows the compositor's activation state itself: focused it is at full
strength and frosted, unfocused it steps back and costs the compositor
nothing to composite.

## Goals

- Which window has the focus is legible from the window's chrome alone, without
  reading its content or comparing it against another window.
- An unfocused window is cheaper for the compositor than a focused one.
- The behaviour belongs to the toolkit, so every kit application gets the same
  treatment without implementing it again.
- A window is never left translucent with nothing behind it to be translucent
  over.

## Non-Goals

- Compositor-drawn title bars — see [window-decorations](window-decorations.md).
- Dimming the window's *content*. A background window's file list, text or
  document stays exactly as readable as it was; only the window's own chrome
  changes.
- A per-application opt-out. An application chooses whether it wants a blurred
  backdrop at all, not whether focus affects it.
- Animating the change. Focus moves discretely and so does the appearance.

## Behavior

**Tracking.** A window's focus state is whatever the compositor's most recent
configure said (`activated`). It is per window, not per process: an application
showing two windows has at most one of them focused, and each follows its own
configures.

**Server-decorated windows get the same treatment**, decided compositor-side
from the keyboard focus rather than from a configure, and drawn by the same
components. A window with Otto's own title bar and a kit window side by side
must not disagree about what an unfocused window looks like — including the
blur: an unfocused title bar stops blurring what is behind it and is filled in
to full opacity, exactly as a client's own panels are. The compositor
additionally lightens an unfocused window's drop shadow, which a client cannot
do for itself.

**Focused.** The window is at full strength:

- The title is drawn in the primary text colour, secondary text in the
  secondary colour.
- Window controls are drawn in their colours — accent-tinted traffic lights —
  and reveal their glyphs while the pointer is over the group.
- If the application asked for a blurred backdrop, the compositor is blurring
  behind the window, and the window's materials are translucent over it.

**Unfocused.** Each of those steps back:

- Title and secondary text each drop one step down the text scale: primary
  becomes secondary, secondary becomes tertiary.
- Window controls go gray — a lighter gray on a dark titlebar than on a light
  one, so they read against either — and reveal no glyphs on hover.
- The blurred backdrop is not requested.

**Materials follow the blur, not the focus.** A translucent material is
translucent *over the blur*; over the bare desktop it is the wallpaper showing
through, which drags down the contrast of everything drawn on top of it. So
whenever there is no blur behind the window — it is unfocused, the application
never asked for one, or the compositor cannot provide one — every translucent
panel material is drawn at full opacity in the same colour instead.

**The change is a fade, not a cut.** The material moves between its
translucent and its filled-in form over about a third of a second rather than
snapping, and the blur is switched at the *opaque* end of that fade in both
directions — whether the material belongs to the window, to the compositor's
own title bar, or to panels an application composites for itself — so the frost is never seen arriving or leaving, only the tint
thinning to reveal it or thickening to cover it. Losing focus therefore keeps
blurring until the material is fully opaque; gaining it turns the blur on
before the material has started to thin. A window the compositor decorates
fades on the same schedule as one that draws its own bar.

An application that composites its own materials owns their fade, and with it
the moment the blur may be dropped — it says when. Turning the blur *on* stays
the window's, since focus is known before the frame in which those materials
start to thin, and waiting to be told would cost exactly the frame the
atomicity rule below forbids.

**Restoring.** An application states once that it wants a blurred backdrop. The
window drops and restores that blur as focus comes and goes, without the
application asking again.

**Atomicity.** No frame may show a translucent material over an unblurred
desktop. Where the material changes in one step — a client's own panels
repainting — that step and the blur change reach the compositor in the same
commit. Where it fades, the ordering above is what guarantees it instead: the
blur only ever turns off under a material that has already finished filling
in, and only ever turns on under one that has not yet started to thin.

## Constraints & Edge Cases

- **A window's own popup must not unfocus it.** A menu or a context menu takes
  the keyboard, and the compositor attributes that focus to the window that
  opened the popup; a window that dimmed itself whenever one of its menus
  opened would be wrong every time it was used.
- **A window that maps while another application has focus comes up
  unfocused.** Nothing is drawn before the first configure, so the first frame
  already carries the right state and there is no focused-looking flash.
- **No style protocol means no blur, ever.** Under a compositor that does not
  carry Otto's surface style, the request is silently unavailable — the
  materials must be filled in for the whole run rather than only while the
  window is unfocused.
- **Controls stay live while unfocused.** Gray is a colour, not a disabled
  state: a press on a background window's close control still closes it.
- **A window with no traffic lights is unaffected by that part.** A dialog that
  is dismissed by its own buttons — the file picker — has no controls to gray.

## Rationale

- **Why the blur is toggled under an opaque cover.** The toggle itself is a
  step change — a surface either has a blurred backdrop or it does not, and
  there is no half a gaussian. Fading the material without moving the toggle
  to the opaque end of the fade just makes that step visible for longer:
  the frost pops in against a half-thinned tint. Hiding the step under the
  one moment when nothing can be seen through the material is what makes the
  whole transition read as a single fade.
- **Why drop the blur rather than dim it.** A full-window gaussian every frame
  is the most expensive thing the compositor does on a window's behalf, and it
  is spent on a window nobody is looking at. Dropping it is both the cheaper
  and the more legible option, and it makes the depth cue and the cost saving
  the same change.
- **Why fill the materials in.** This was found the hard way: dropping the blur
  alone left the desktop showing through the sidebar and header, and the
  header's own buttons and view switcher lost contrast against the wallpaper
  behind them. The translucency was never the point — the frost was.
- **Why a step down the text scale rather than a blanket opacity.** Reducing
  the whole titlebar's alpha fades the material along with the text and makes
  the window look faulty. Moving each text tone one step down the scale keeps
  every element at a tone the theme actually defines.

## Open Questions

- macOS reveals the traffic-light glyphs on hover even in a background window,
  which makes closing one a single click without focusing it first. Kit windows
  currently reveal them only while focused. Worth matching?
