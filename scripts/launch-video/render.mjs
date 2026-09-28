// Renders film.html to an MP4, or to JPEG stills for checking.
//
//   node render.mjs stills [out-dir] <seconds...>   e.g. stills out 4.5 10 26.5
//   node render.mjs video  [out.mp4] [duration]     default out/otto-launch.mp4, 45
//
// Time is virtual: the page's requestAnimationFrame and performance.now only
// advance when a frame is requested, so every render is identical and frame
// exact regardless of how slow the machine is.
import { chromium } from "playwright-core";
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname } from "node:path";
import ffmpeg from "ffmpeg-static";

const FPS = 30;
const [mode = "stills", outArg, ...rest] = process.argv.slice(2);
if (!existsSync("build/fonts.css")) throw new Error("build/ is missing: run `bash prepare.sh` first");

const html = readFileSync("film.html", "utf8")
  .replace("{{fonts}}", () => readFileSync("build/fonts.css", "utf8"))
  .replace(/\{\{img:([\w-]+)\}\}/g, (_, n) => "data:image/jpeg;base64," + readFileSync(`build/img/${n}.jpg`).toString("base64"));

// The page runs rAF callbacks only when __seek(ms) is called.
const CLOCK = `(() => {
  let now = 0, rafs = [];
  performance.now = () => now;
  window.requestAnimationFrame = cb => rafs.push(cb);
  window.cancelAnimationFrame = () => {};
  window.__seek = t => { now = t; const cbs = rafs; rafs = []; for (const cb of cbs) cb(now); };
})();`;

// Chromium's executable: PLAYWRIGHT_CHROMIUM, else Playwright's own lookup.
const browser = await chromium.launch({
  executablePath: process.env.PLAYWRIGHT_CHROMIUM || undefined,
  args: ["--no-sandbox", "--disable-gpu", "--font-render-hinting=none", "--hide-scrollbars", "--force-color-profile=srgb"],
});
const ctx = await browser.newContext({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
await ctx.addInitScript(CLOCK);
const page = await ctx.newPage();
// Clip frames are served from build/seq so the page never holds them all at once.
await page.route("http://clips.local/**", route =>
  route.fulfill({ contentType: "image/jpeg", body: readFileSync("build/seq/" + new URL(route.request().url()).pathname.slice(1)) }));
const errors = [];
page.on("pageerror", e => errors.push(e.message));
page.on("console", m => m.type() === "error" && errors.push(m.text()));
await page.setContent(html, { waitUntil: "load", timeout: 60_000 });
await page.evaluate(() => document.fonts.ready);

const seekTo = ms => page.evaluate(async t => { window.__seek(t); await window.__syncVideos(); }, ms);

if (mode === "stills") {
  const dir = outArg || "out";
  mkdirSync(dir, { recursive: true });
  for (const s of rest) {
    await seekTo(Number(s) * 1000);
    await page.screenshot({ path: `${dir}/t${s}.jpg`, type: "jpeg", quality: 75 });
  }
} else {
  const out = outArg || "out/otto-launch.mp4";
  mkdirSync(dirname(out), { recursive: true });
  const total = Math.round(Number(rest[0] || 45) * FPS);
  const ff = spawn(ffmpeg, ["-y", "-loglevel", "error", "-f", "image2pipe", "-framerate", String(FPS), "-i", "-",
    "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p", "-movflags", "+faststart", out]);
  const done = new Promise(r => ff.on("close", r));
  for (let i = 0; i < total; i++) {
    await seekTo((i * 1000) / FPS);
    const frame = await page.screenshot({ type: "jpeg", quality: 92 });
    if (!ff.stdin.write(frame)) await new Promise(r => ff.stdin.once("drain", r));
  }
  ff.stdin.end();
  if ((await done) !== 0) errors.push("ffmpeg failed");
}
await browser.close();
if (errors.length) { console.error("page errors:\n" + errors.join("\n")); process.exit(1); }
