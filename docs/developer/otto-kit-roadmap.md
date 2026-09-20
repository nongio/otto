# otto-kit: application UI

What otto-kit gives an app today, what is still missing, and the order the
remaining pieces are being built in.

> **Status: partially built.** The gap analysis is kept against the settings
> app ([specs/settings-app.md](../../specs/settings-app.md)), the first otto-kit
> app that is mostly *forms* rather than mostly custom drawing. The sections
> below mark what is done and what is not.

## Two shapes of component

otto-kit is a drawing library with a Wayland shell attached: a window, a
surface, a theme, typography, icons, and components that know how to paint
themselves.

```rust
Label::new("Cursor size").with_style(styles::SUBHEADLINE).render(canvas);
```

Most components are stateless builders implementing `Renderable`.
`render(&self, canvas)` takes `&self`, draws, and returns nothing. That works
well where the app already owns a model and draws a bespoke view of it: the
dock, the bar, notifications, the launcher.

It works badly for a form. A form is dozens of small interactive controls, and
a stateless builder gives a control no way to know it was clicked.

**`TextInput` is the model for the other shape.** It has a state struct, a
`render_at(canvas, w, h)`, `on_pointer_down` / `on_pointer_drag` /
`on_pointer_up`, an `on_key` returning a `TextInputResponse`, and an
`offset_at(x)` for hit-testing. Retained state, immediate-mode draw, event
methods returning responses: that is the shape an interactive control needs.

## Who consumes what

The apps and the compositor use otto-kit in opposite ways, and that split
constrains every design decision below.

**The client apps use a narrow slice of the components layer.** otto-bar takes
`menu_bar`, `context_menu` and `menu_item`; otto-launcher and otto-emoji take
`text_input` and `scroll`; otto-settings and otto-files take the form
primitives, `scroll`, `titlebar` and `window`. otto-islands, otto-lock,
otto-greeter and otto-auth-ui take only the infrastructure (`AppRunner`,
`AppContext`, surfaces, protocols, icons, typography, theme, lottie) and draw
their own Skia directly. `Window` has no consumer outside the examples,
otto-settings and otto-files; every other app is layer-shell, subsurface, or
session-lock.

**The compositor is the components layer's other real user.** `src/` draws
server-side decorations with `WindowDecoration` / `WindowControl`, its menus
with `ContextMenuRenderer`, and reads `theme::Theme`, `typography::styles` and
`icons::*`. Server-side rendering is not a hypothetical future consumer of this
toolkit. It is the one shipping today.

## The constraint: stay drawable from the compositor

Otto draws server-side components into a Skia canvas inside a `lay-rs` draw
closure. There is no `AppRunner`, no `AppContext`, no `wl_surface`. That path
is what keeps compositor-drawn UI looking like app-drawn UI.

So **a component must have a form that paints from a canvas plus explicit
state**, with interaction sitting on top rather than in the way. A component
whose only entry point requires the client runtime cannot be used by the
compositor, and the shared look quietly stops being shared.

One lookup breaks that rule: `icons::named_icon_sized` calls
`AppContext::scale_factor()`, so the compositor uses `cached_file_icon` and
`find_icon_in_theme` instead, routing around a client-runtime dependency that
sits inside what should be a pure lookup.

## Infrastructure

**1. Draw + hit-test convention: done.** A component that reports no geometry
makes an app compute rects twice: once to draw, once to test, and the two
drift.

The convention: **a component ships a draw function, and a hit-test helper
where one is needed**, both reading the same geometry. The helper answers "what
is under this point" (an index, a part, or nothing), and the caller decides
what that means.

```rust
let layout = SourceListLayout::compute(items.len(), x, y, width);
source_list::draw(canvas, &layout, &items, selected, &theme, icon);
layout.item_at(px, py) // -> Option<usize>
```

State stays with the caller, so the compositor can draw a component with none
of the interaction machinery and an app can build interaction on top without
re-deriving layout. In the toolkit today: `SourceListLayout::item_at`,
`ListLayout::row_at`, `slider::value_at`, `color_picker::{sv_at, hue_at,
swatch_at, mode_at, hex_field_at}`, `toggle::hit_test`,
`dropdown::field::hit_test`, `TextInput::offset_at`. Static components
(`Label`) correctly have none.

**2. Interaction state: partly done.** `Slider` and `ScrollView` own their
pointer state (`on_pointer_down` / `_drag` / `_up`). `Toggle` carries
`ToggleInteraction` (`Normal`/`Hovered`/`Pressed`/`Disabled`) with `hit_test`
and an animated `Flip`, but the caller still sets the state from its own
pointer tracking. `Button` is in the same position with a caller-set
`ButtonState`. Hover, press and disabled ultimately belong to the widget.

**3. A scroll view: done.** `components/scroll/` clips content, holds an
offset, draws a scrollbar, and handles `on_wheel` alongside pointer drag. Axis
events reach apps already: `pointer_frame` forwards raw `PointerEvent`s
untouched.

It also owns the *feel*: `ScrollView` samples input into a velocity and
coasts under friction after the gesture ends, resists past either end and
springs back (a fling into an end bounces off it), and fades its overlay
scrollbar in while scrolling, out when idle, and wider under the pointer. A
host drives all of that by calling `ScrollView::tick` each loop iteration
while `is_animating()`. See `on_update`/`idle_timeout` in `otto-settings`.
Continuous sources go to `on_wheel`; a notched wheel (`discrete != 0`) goes to
`on_wheel_discrete`, which steps without flinging.

The same module holds `ScrollPane`, which scrolls by moving a subsurface band
rather than repainting the window, plus `RowLayout` and `GridLayout` for
closed-form geometry. See [Scroll panes](scroll-pane.md).

**4. Focus and keyboard navigation: done.** `components/otto-kit/src/focus.rs`
is the shared layer: `FocusId` keys a control, `FocusRing` collects the
focusable entries a frame registered and moves focus between them
(`FocusMove`), and `draw_focus_ring` paints the indicator. It is re-exported
from `lib.rs` and used by `otto-settings`, `otto-launcher`, `otto-bar`,
`otto-islands`, `otto-files`, `otto-emoji` and the accessibility tree, which
reads the same registration to report focus over AT-SPI.

**5. Popup anchoring from inside a window: done.** `components/dropdown/`
has the path: `dropdown::menu::DropdownMenu` builds an `XdgPositioner` from a
field's rect, drives a reused `ContextMenu`, and reports the chosen index back.
`context_menu.rs` registers its own pointer callback per instance and assumes
it is the whole interaction while up, but a `DropdownMenu` keeps one
`ContextMenu` for the dropdown's whole lifetime, **built once up front, never
inside a pointer-event handler** (doing it lazily deadlocks `AppContext`'s
callback list; confirmed live). So multiple dropdowns on one window do not
collide and nothing leaks per click. `ContextMenu` has an `on_close` hook, so a
dropdown's field can notice a dismissal (ESC, click outside) it did not
initiate; callers that never set it are unaffected. Full account in
`dropdown::menu`'s module docs.

## The widgets

**Form primitives**, the largest single win, since every settings pane is
built from them:

| Widget | Status |
|--------|--------|
| Select / pop-up button | Done: `components/dropdown/` |
| Slider | Done: `components/slider/`, with `value_at` |
| Sidebar / source list | Done: `components/source_list/`, with `item_at` |
| Grouped list / rows | Done: `components/list/`, with `row_at` |
| Colour well and picker | Done: `components/color_picker/` |
| Toggle switch | Done: `components/toggle/`, with `hit_test` and an animated flip |
| Search field | Open: `TextInput` plus field chrome and a clear button |

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
the list above: drawn stateless, positioned by hardcoded rects, inert. It
exists to look at, and should be promoted into the toolkit rather than written
twice.

## Order of work

1. Interaction state owned by `Button` rather than set by the caller.
2. Remaining specialised widgets, pulled in by whichever pane needs them.

The display arrangement canvas — outputs as draggable rectangles with edge
snapping — is deliberately *not* a reusable control and belongs to the settings
app.

## Draw functions by default, layers by exception

Components are draw functions. `Renderable::to_layer` exists, and the
compositor's own UI is built on `lay-rs` layers, but that is not the default a
component should reach for.

A draw function is just a call: the caller keeps the state, the compositor can
paint the component without hosting anything, and there is no tree to reconcile
against Otto's own scene graph. A layer is retained state, and every component
that becomes one is state somebody has to own, mount, and keep in sync.

**Use a layer only when it earns it**: as an optimisation (the content is
expensive and mostly static, so caching its picture beats repainting) or for an
effect (it needs animation, transform, blur, or opacity compositing, which a
canvas cannot give). Neither is a property of a widget in the abstract, so the
choice belongs to whoever places the component, not to its definition.

## Open question

Why do the client apps use so little of the components layer? Apps that could
use `Label` or `Button` write raw Skia instead, while the compositor adopted
`WindowDecoration` and `ContextMenuRenderer` readily.

If the answer is "they were unusable without hit-testing and state", the
priorities above are right and the finished work should keep showing up as
adoption. If it is "the builder API doesn't give apps the control they need",
more widgets will not fix it and the API shape is the thing to change. Worth
checking against otto-settings before building the remaining widgets in the
same style.
