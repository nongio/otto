## otto-kit: application UI

> **Status: partially built.** Written while scaffolding the settings app
> ([specs/settings-app.md](../../specs/settings-app.md)) — the first otto-kit
> app that is mostly *forms* rather than mostly custom drawing, and so the
> first to hit these gaps. Since then the draw/hit-test convention, the scroll
> view and most of the form primitives have landed. The sections below mark
> what is done; the analysis is kept because it is what the remaining work is
> being built against.

### Where the toolkit started

otto-kit was a drawing library with a Wayland shell attached: a window, a
surface, a theme, typography, icons, and components that know how to paint
themselves.

```rust
Label::new("Cursor size").with_style(styles::SUBHEADLINE).render(canvas);
```

Components were stateless builders implementing `Renderable`. `render(&self,
canvas)` takes `&self`, draws, and returns nothing. That works well for what
otto-kit was built for — the dock, the bar, notifications, the launcher —
where the app already owns a model and draws a bespoke view of it.

It works badly for a form. A form is dozens of small interactive controls, and
a stateless builder gives a control no way to know it was clicked.

**`TextInput` was the exception, and became the precedent.** It has a state
struct, a `render_at(canvas, w, h)`, `on_pointer_down` / `on_pointer_drag` /
`on_pointer_up`, an `on_key` returning a `TextInputResponse`, and an
`offset_at(x)` for hit-testing. Retained state, immediate-mode draw, event
methods returning responses — that is the shape the rest of the toolkit needed.

### Who consumes what

The apps and the compositor use otto-kit in opposite ways, and that split
constrains every design decision below.

**The client apps barely touch the components layer.** otto-bar uses `MenuBar`
and `ContextMenu`; otto-launcher uses `TextInput`; otto-settings uses the form
primitives. Everything else — otto-islands, otto-lock, otto-greeter,
otto-auth-ui — takes only the infrastructure (`AppRunner`, `AppContext`,
surfaces, protocols, icons, typography, theme, lottie) and draws its own Skia
directly. `Window` itself has almost no consumer outside examples and the
settings app; every other app is layer-shell, subsurface, or session-lock.

**The compositor is the components layer's other real user.** `src/` draws
server-side decorations with `Titlebar` / `WindowControl`, its menus with
`ContextMenuRenderer`, and reads `theme::Theme`, `typography::styles` and
`icons::*`. Server-side rendering is not a hypothetical future consumer of this
toolkit — it is the one shipping today.

### The constraint: stay drawable from the compositor

Otto draws server-side components into a Skia canvas inside a `lay-rs` draw
closure. There is no `AppRunner`, no `AppContext`, no `wl_surface`. That path
is what keeps compositor-drawn UI looking like app-drawn UI.

So **a component must have a form that paints from a canvas plus explicit
state**, with interaction sitting on top rather than in the way. A component
whose only entry point requires the client runtime cannot be used by the
compositor, and the shared look quietly stops being shared.

One instance of this already went wrong: `icons::named_icon_sized` calls
`AppContext::scale_factor()`, so the compositor uses `cached_file_icon` and
`find_icon_in_theme` instead — routing around a client-runtime dependency that
ended up inside what should be a pure lookup.

### Infrastructure

**1. Draw + hit-test convention — done.** The problem was that no component
reported its own geometry, so an app computed rects twice: once to draw, once
to test. The settings scaffold had `sidebar_item_rect(i)` shared by the
renderer and by `pane_at(x, y)` precisely so the two could not drift.

The convention that was standardised: **a component ships a draw function, and
a hit-test helper where one is needed**, both reading the same geometry. The
helper answers "what is under this point" — an index, a part, or nothing — and
the caller decides what that means.

```rust
source_list::draw(canvas, rect, &items, selected, &theme);
source_list::item_at(rect, x, y) -> Option<usize>
```

State stays with the caller, so the compositor can draw a component with none
of the interaction machinery and an app can build interaction on top without
re-deriving layout. In the toolkit today: `source_list::item_at`,
`list::row_at`, `slider::value_at`, `color_picker::{sv_at, hue_at, swatch_at,
mode_at, hex_field_at}`, `text_input::offset_at`. Static components (`Label`)
correctly have none.

**2. Interaction state — partly done.** `Slider` and `Scroll` own their pointer
state (`on_pointer_down` / `_drag` / `_up`). `Button` still has a
`ButtonState` the *caller* sets, and `Toggle` has only a `draw` — neither can
highlight on hover without the app writing pointer plumbing. Hover, press and
disabled belong to the widget.

**3. A scroll view — done.** `components/scroll/` clips content, holds an
offset, draws a scrollbar, and handles `on_wheel` alongside pointer drag. Axis
events already reached apps (`pointer_frame` forwards raw `PointerEvent`s
untouched); the missing piece was the component.

**4. Focus and keyboard navigation — open.** There is still no focus ring, no
tab order, and no shared notion of which control has keyboard focus.
Individual components track their own (`TextInput`, `SourceList`, `MenuBar`),
but nothing coordinates them. The settings spec commits to the app being usable
from the keyboard alone, so this is a requirement, not polish. **This is the
main remaining infrastructure gap.**

**5. Popup anchoring from inside a window — done.** `components/dropdown/`
has the path: `dropdown::menu::DropdownMenu` builds an `XdgPositioner` from a
field's rect, drives a reused `ContextMenu`, and reports the chosen index back.
`context_menu.rs` still registers its own pointer callback per instance and
still assumes it is the whole interaction while up — that did not change — but
a `DropdownMenu` keeps one `ContextMenu` for the dropdown's whole lifetime,
**built once up front, never inside a pointer-event handler** (doing it lazily
deadlocks `AppContext`'s callback list — confirmed live). So multiple dropdowns
on one window do not collide and nothing leaks per click. `ContextMenu` gained
one additive hook, `on_close`, so a dropdown's field can notice a dismissal
(ESC, click outside) it did not initiate; callers that never set it are
unaffected. Full account in `dropdown::menu`'s module docs.

### The widgets

**Form primitives** — the largest single win, since every settings pane is
built from them:

| Widget | Status |
|--------|--------|
| Select / pop-up button | Done — `components/dropdown/` |
| Slider | Done — `components/slider/`, with `value_at` |
| Sidebar / source list | Done — `components/source_list/`, with `item_at` |
| Grouped list / rows | Done — `components/list/`, with `row_at` |
| Colour well and picker | Done — `components/color_picker/` |
| Toggle switch | Draws; no interaction state yet |
| Search field | Open — `TextInput` plus field chrome and a clear button |

**Specialised**, each driven by one settings pane:

| Widget | Driven by | Status |
|--------|-----------|--------|
| Scrollbar | the scroll view | Done |
| Key-capture field | Keyboard shortcuts. Needs a keyboard grab so the captured combination does not also reach the compositor. | Open |
| Segmented control | Light/Dark and similar short exclusive choices | Open |
| Stepper / numeric field | Timeouts, sizes | Open |
| Disclosure group | Collapsing secondary settings | Open |
| Status pill | "Restart required" and similar row states | Open |

Whatever remains in `components/otto-settings/src/widgets.rs` is a prototype of
the list above — drawn stateless, positioned by hardcoded rects, inert. It
exists to look at, and should be promoted into the toolkit rather than written
twice.

### Order of work

1. ~~Draw/hit-test convention, scroll view~~ — done.
2. **Focus and keyboard navigation** — the remaining blocker. Every further
   widget's shape depends on the answer.
3. Interaction state for `Button` and `Toggle`.
4. Remaining specialised widgets, pulled in by whichever pane needs them.

The display arrangement canvas — outputs as draggable rectangles with edge
snapping — is deliberately *not* a reusable control and belongs to the settings
app.

### Draw functions by default, layers by exception

Components are draw functions. `Renderable::to_layer` exists, and the
compositor's own UI is built on `lay-rs` layers, but that is not the default a
component should reach for.

A draw function is just a call: the caller keeps the state, the compositor can
paint the component without hosting anything, and there is no tree to reconcile
against Otto's own scene graph. A layer is retained state, and every component
that becomes one is state somebody has to own, mount, and keep in sync.

**Use a layer only when it earns it** — as an optimisation (the content is
expensive and mostly static, so caching its picture beats repainting) or for an
effect (it needs animation, transform, blur, or opacity compositing, which a
canvas cannot give). Neither is a property of a widget in the abstract, so the
choice belongs to whoever places the component, not to its definition.

### Open question

Why did the client apps skip the components layer? Apps that could have used
`Label` or `Button` wrote raw Skia instead, while the compositor adopted
`Titlebar` and `ContextMenuRenderer` readily.

If the answer is "they were unusable without hit-testing and state", the
priorities above are right and the recent work should show up as adoption. If
it is "the builder API doesn't give apps the control they need", more widgets
will not fix it and the API shape is the thing to change. Worth checking
against otto-settings before building the remaining widgets in the same style.
