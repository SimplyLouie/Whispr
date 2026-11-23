// Draws Whispr's logo and renders every icon made from it: the app icon set
// (src-tauri/icons), the tray icons (src-tauri/resources) and the browser
// extension's icons. The vector sources are written to branding/.
//
//   bun scripts/build-brand-icons.ts            render everything
//   bun scripts/build-brand-icons.ts --preview <dir>
//                                               only a contact sheet, in <dir>
//
// The in-app drawings (src/components/icons) repeat the same shapes by hand;
// change them together.

import { spawnSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";
import { fileURLToPath } from "url";

const ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");

// ---------- the shapes, on a 64 x 64 canvas ----------

/** The mark: a quiet sound wave that reads as a "w". */
const WAVE =
  "M8 21 C12.5 21 15 43 20.5 43 C26 43 27 26.5 32 26.5 C37 26.5 38 43 43.5 43 C49 43 51.5 21 56 21";
const WAVE_STROKE = 6.5;

const VIOLET = "#7f6df2";
const VIOLET_LIGHT = "#a797fb";
const GRADIENT_FROM = "#a394ff";
const GRADIENT_TO = "#5a43e0";

const wave = (color: string, extra = "") =>
  `<path d="${WAVE}" fill="none" stroke="${color}" stroke-width="${WAVE_STROKE}" stroke-linecap="round" stroke-linejoin="round" ${extra}/>`;

const svg = (size: number, body: string, viewBox = "0 0 64 64") =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="${viewBox}">${body}</svg>\n`;

/** A filled disc with `cutout` (drawn in black) removed from it. */
const disc = (color: string, cutout: string, id: string) =>
  `<mask id="${id}"><rect width="64" height="64" fill="#fff"/>${cutout}</mask>` +
  `<circle cx="32" cy="32" r="29" fill="${color}" mask="url(#${id})"/>`;

type TrayState = "idle" | "recording" | "transcribing" | "warning";

/** One tray icon's drawing in a single color. */
function tray(state: TrayState, color: string): string {
  switch (state) {
    case "idle":
      return wave(color);
    case "recording":
      // Listening: the wave, on a disc.
      return disc(
        color,
        wave(
          "#000",
          'transform="translate(32 32) scale(0.68) translate(-32 -32)"',
        ),
        "m",
      );
    case "transcribing":
      // Writing: an ellipsis, on a disc.
      return disc(
        color,
        [19, 32, 45]
          .map((x) => `<circle cx="${x}" cy="32" r="4.6" fill="#000"/>`)
          .join(""),
        "m",
      );
    case "warning":
      // The wave, smaller, with an exclamation badge beside it.
      return (
        wave(color, 'transform="translate(-2 -6) scale(0.82)"') +
        `<mask id="b"><rect width="64" height="64" fill="#fff"/>` +
        `<path d="M47 37.5 V47.5" stroke="#000" stroke-width="4.4" stroke-linecap="round"/>` +
        `<circle cx="47" cy="54.2" r="2.5" fill="#000"/></mask>` +
        `<circle cx="47" cy="46" r="14.5" fill="${color}" mask="url(#b)"/>`
      );
  }
}

/** The app icon: the mark on a violet tile, with the margin macOS expects. */
const APP_ICON =
  `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">` +
  `<defs>` +
  `<linearGradient id="tile" x1="0.15" y1="0" x2="0.85" y2="1">` +
  `<stop offset="0" stop-color="${GRADIENT_FROM}"/><stop offset="1" stop-color="${GRADIENT_TO}"/></linearGradient>` +
  `<linearGradient id="sheen" x1="0" y1="0" x2="0" y2="1">` +
  `<stop offset="0" stop-color="#fff" stop-opacity="0.22"/><stop offset="0.55" stop-color="#fff" stop-opacity="0"/></linearGradient>` +
  `</defs>` +
  `<rect x="100" y="100" width="824" height="824" rx="186" fill="url(#tile)"/>` +
  `<rect x="100" y="100" width="824" height="824" rx="186" fill="url(#sheen)"/>` +
  `<g transform="translate(512 512) scale(10.4) translate(-32 -32)">${wave("#fff")}</g>` +
  `</svg>\n`;

/** The mark alone, in the brand color. */
const MARK = svg(512, wave(VIOLET));

// ---------- rendering ----------

/** Renders `source` (an SVG file) to a `size` px PNG at `target`. */
function renderPng(source: string, size: number, target: string) {
  const out = fs.mkdtempSync(path.join(os.tmpdir(), "whispr-icon-"));
  run(["icon", source, "--png", String(size), "-o", out]);
  fs.copyFileSync(path.join(out, `${size}x${size}.png`), target);
  fs.rmSync(out, { recursive: true });
}

function run(args: string[]) {
  const result = spawnSync("bun", ["run", "tauri", ...args], {
    cwd: ROOT,
    stdio: ["ignore", "ignore", "inherit"],
  });
  if (result.status !== 0) {
    throw new Error(`tauri ${args.join(" ")} failed`);
  }
}

function writeTemp(name: string, content: string): string {
  const file = path.join(
    fs.mkdtempSync(path.join(os.tmpdir(), "whispr-svg-")),
    name,
  );
  fs.writeFileSync(file, content);
  return file;
}

/** Every tray icon file and the drawing in it. */
const TRAY_FILES: [file: string, state: TrayState, color: string][] = [
  // Light icons, for a dark menu bar or tray (and macOS templates).
  ["tray_idle.png", "idle", "#fff"],
  ["tray_recording.png", "recording", "#fff"],
  ["tray_transcribing.png", "transcribing", "#fff"],
  ["tray_idle_warning.png", "warning", "#fff"],
  // Dark icons, for a light one.
  ["tray_idle_dark.png", "idle", "#000"],
  ["tray_recording_dark.png", "recording", "#000"],
  ["tray_transcribing_dark.png", "transcribing", "#000"],
  ["tray_idle_warning_dark.png", "warning", "#000"],
  // Colored icons, where the tray's shade isn't known (Linux).
  ["handy.png", "idle", VIOLET_LIGHT],
  ["recording.png", "recording", VIOLET_LIGHT],
  ["transcribing.png", "transcribing", VIOLET_LIGHT],
  ["handy_warning.png", "warning", VIOLET_LIGHT],
];

function preview(dir: string) {
  fs.mkdirSync(dir, { recursive: true });
  const cell = (x: number, y: number, bg: string, body: string) =>
    `<g transform="translate(${x} ${y})"><rect width="128" height="128" fill="${bg}"/>` +
    `<svg x="24" y="24" width="80" height="80" viewBox="0 0 64 64">${body}</svg></g>`;
  const uniq = (body: string, n: number) =>
    body.replace(/\b(m|b)\b(?=["\)])/g, `$1${n}`);
  const cells = TRAY_FILES.map(([, state, color], i) =>
    cell(
      (i % 4) * 128,
      512 + Math.floor(i / 4) * 128,
      i < 4 ? "#26262a" : i < 8 ? "#ececf0" : "#55555c",
      uniq(tray(state, color), i),
    ),
  ).join("");
  const sheet =
    `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 512 896">` +
    `<rect width="512" height="896" fill="#f4f4f6"/>` +
    `<svg x="0" y="0" width="512" height="512" viewBox="0 0 1024 1024">${APP_ICON.replace(/^<svg[^>]*>|<\/svg>\n$/g, "")}</svg>` +
    cells +
    `</svg>`;
  renderPng(writeTemp("sheet.svg", sheet), 1024, path.join(dir, "preview.png"));
  // The sizes that have to survive being small.
  for (const size of [16, 32]) {
    renderPng(
      writeTemp("icon.svg", APP_ICON),
      size,
      path.join(dir, `app-${size}.png`),
    );
  }
}

function build() {
  const branding = path.join(ROOT, "branding");
  fs.mkdirSync(branding, { recursive: true });
  fs.writeFileSync(path.join(branding, "whispr-icon.svg"), APP_ICON);
  fs.writeFileSync(path.join(branding, "whispr-mark.svg"), MARK);

  // The whole app icon set: png sizes, icns, ico, and the mobile ones.
  run(["icon", path.join(branding, "whispr-icon.svg")]);
  // Not part of that set, but kept next to it at full size.
  renderPng(
    path.join(branding, "whispr-icon.svg"),
    1024,
    path.join(ROOT, "src-tauri", "icons", "logo.png"),
  );

  const resources = path.join(ROOT, "src-tauri", "resources");
  for (const [file, state, color] of TRAY_FILES) {
    renderPng(
      writeTemp("tray.svg", svg(64, tray(state, color))),
      64,
      path.join(resources, file),
    );
  }

  const extension = path.join(ROOT, "chrome-extension", "icons");
  for (const size of [32, 128]) {
    fs.copyFileSync(
      path.join(
        ROOT,
        "src-tauri",
        "icons",
        size === 32 ? "32x32.png" : "128x128.png",
      ),
      path.join(extension, `${size}.png`),
    );
  }
}

const previewDir = process.argv.indexOf("--preview");
if (previewDir !== -1) {
  preview(process.argv[previewDir + 1] ?? "brand-preview");
} else {
  build();
}
