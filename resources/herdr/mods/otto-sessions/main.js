// Agent sessions as Otto's Ask lists them: a status dot, the session's title,
// and "@agent · status · folder" under it.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { agents, panes, run, theme, exec, appHome, checking, readTextFile, writeTextFile } from "herdr";

// What an agent puts before its title: a status glyph or spinner and the separator after it.
const clean = (s) => (s ?? "").replace(/^[^\p{L}\p{N}\p{Ps}\p{Pi}]+/u, "").replace(/^\[ [!.] \] /, "").trim();

const tilde = (path) => (path ?? "").replace(/^\/home\/[^/]+|^\/Users\/[^/]+|^\/root/, "~");

// Needs you first, then working, then the rest; most recently changed first within each.
const RANK = { blocked: 3, working: 2, done: 1, idle: 0 };
const WORD = { blocked: "needs input", working: "working", done: "done", idle: "idle" };
const SECTION = { blocked: "Needs input", working: "Working", done: "Done", idle: "Idle" };
const ORDER = ["blocked", "working", "done", "idle"];

// Otto's accent, as the desktop portal gives it: "#rrggbb", or null where there is none.
async function portalAccent() {
  const out = await exec("gdbus", ["call", "--session", "-d", "org.freedesktop.portal.Desktop", "-o", "/org/freedesktop/portal/desktop",
    "-m", "org.freedesktop.portal.Settings.ReadOne", "org.freedesktop.appearance", "accent-color"]);
  const rgb = out.stdout.match(/[-\d.e]+/g)?.map(Number);
  if (out.code !== 0 || rgb?.length !== 3 || rgb.some((c) => !(c >= 0 && c <= 1))) return null;
  return "#" + rgb.map((c) => Math.round(c * 255).toString(16).padStart(2, "0")).join("");
}

// Writes the accent into the Otto themes, only where it differs: herdr reloads them live.
async function applyAccent(accent) {
  for (const name of ["otto-dark", "otto-light"]) {
    const path = `${appHome()}/themes/${name}.json`;
    const text = await readTextFile(path);
    if (!text) continue;
    const theme = JSON.parse(text);
    theme.colors ??= {};
    if (theme.colors.accent === accent && theme.cursor === accent) continue;
    theme.cursor = accent;
    theme.colors.accent = accent;
    theme.colors.solid = accent;
    await writeTextFile(path, JSON.stringify(theme, null, 2) + "\n", text);
  }
}

// Returns when the portal announces a new accent, or after about ten minutes.
const ACCENT_CHANGED = "timeout 590 dbus-monitor --session " +
  "\"type='signal',interface='org.freedesktop.portal.Settings',member='SettingChanged',arg0='org.freedesktop.appearance',arg1='accent-color'\" " +
  "| { grep -m1 -q member=SettingChanged; pkill -P $$ -x timeout; }";

// Row metrics from otto-kit's item list.
const ROW_H = 46;
const DOT = 8;

export default class Sessions extends View {
  init(_props, cx) {
    this.folded = new Set();
    if (checking()) return;
    cx.spawn(async () => {
      for (;;) {
        try {
          const accent = await portalAccent();
          if (accent) await applyAccent(accent);
        } catch (e) {
          console.warn(`otto accent: ${e}`);
        }
        await exec("sh", ["-c", ACCENT_CHANGED], { timeout: 600000 }).catch(() => {});
      }
    });
  }

  // A section's header; a click folds it. Its label wears the dot's colour when it needs you.
  header(t, status, count) {
    const open = !this.folded.has(status);
    return h_flex()
      .id(`h-${status}`)
      .h(28)
      .mt(6)
      .px(10)
      .flex_none()
      .gap(6)
      .items_center()
      .rounded(7)
      .cursor_pointer()
      .text_size(11.5)
      .font_medium()
      .hover((el) => el.bg(t.hover))
      .on_click((_e, cx) => {
        open ? this.folded.add(status) : this.folded.delete(status);
        cx.notify();
      })
      .child(div().text_color(status === "blocked" ? t.ansi3 : t.dim).child(SECTION[status]))
      .child(div().text_color(t.dim).child(String(count)))
      .child(div().flex_1())
      .child(svg(`mods/otto-sessions/chevron-${open ? "down" : "right"}.svg`).size(11).flex_none().text_color(t.dim));
  }

  dot(t, status) {
    const color = status === "working" ? t.accent : status === "blocked" ? t.ansi3 : status === "done" ? t.ansi2 : t.dim;
    return div().size(DOT).flex_none().rounded(DOT / 2).bg(color)
      .when(status === "working", (el) => el.loop("opacity", { from: 0.45, to: 1, period: 1400 }));
  }

  row(t, a, cwd) {
    const name = a.program?.name || a.app || "agent";
    const title = clean(a.title);
    const heading = title && title !== name && !/^[~/]/.test(title) ? title : a.tab_name || a.space_name || name;
    const sub = [`@${name}`, WORD[a.status] ?? a.status, tilde(cwd)].filter(Boolean).join(" · ");
    return h_flex()
      .id(`s-${a.address}`)
      .h(ROW_H)
      .flex_none()
      .rounded(9)
      .items_center()
      .when(a.focused, (el) => el.bg(t.selected))
      .hover((el) => el.bg(t.hover))
      .cursor_pointer()
      .on_click(() => run("pane.focus", { pane: a.pane }))
      .child(h_flex().w(36).flex_none().items_center().justify_center().child(this.dot(t, a.status)))
      .child(v_flex().flex_1().min_w(0).pr(10).gap(1)
        .child(div().truncate().text_size(14).text_color(t.foreground).child(heading))
        .child(div().truncate().text_size(11.5).text_color(t.muted).child(sub)));
  }

  render() {
    const t = theme();
    const cwd = new Map(panes().map((p) => [p.pane, p.cwd]));
    const list = [...agents()].sort((x, y) => (RANK[y.status] ?? 0) - (RANK[x.status] ?? 0) || y.seq - x.seq);
    const rows = ORDER.flatMap((status) => {
      const members = list.filter((a) => (ORDER.includes(a.status) ? a.status : "idle") === status);
      if (!members.length) return [];
      const head = this.header(t, status, members.length);
      return this.folded.has(status) ? [head] : [head, ...members.map((a) => this.row(t, a, cwd.get(a.pane)))];
    });
    return v_flex()
      .size_full()
      .min_h(0)
      .bg(t.chrome)
      .border_t(1)
      .border_color(t.border)
      .child(h_flex().px(16).h(36).pt(10).flex_none().items_center()
        .child(div().text_size(11.5).font_medium().text_color(t.dim).child("Sessions")))
      .when(list.length === 0, (el) =>
        el.child(div().px(16).pt(4).text_size(12).text_color(t.dim).child("No sessions yet.")))
      .child(v_flex().id("s-list").flex_1().min_h(0).px(8).overflow_y_scrollbar()
        .children(rows));
  }
}
