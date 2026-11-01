// The tab's half of `vaayu --web`. The editor itself runs on the server; this
// keeps a cell grid, applies the renderer's escape-sequence frames to it, paints
// changed rows on a canvas and reports input. There is no build step and there
// are no dependencies, so what is here is what the browser runs.
"use strict";

// Unset colours paint with the page theme. Palette themes set their own.
const DEFAULT_FG = 0xd8dee9;
const DEFAULT_BG = 0x14161b;
const NO_COLOR = -1;

const ATTR_BOLD = 1, ATTR_DIM = 2, ATTR_ITALIC = 4, ATTR_UNDERLINE = 8;
const ATTR_REVERSE = 16, ATTR_STRIKE = 32, ATTR_IMAGE = 64;

const FONT_STACK = '"JetBrainsMono Nerd Font", "JetBrainsMono NF", "FiraCode Nerd Font", ' +
  '"Hack Nerd Font", "CaskaydiaCove Nerd Font", "Symbols Nerd Font Mono", "JetBrains Mono", ' +
  '"Fira Code", "Cascadia Code", "SF Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace';

const CLUSTER_BASE = 0x200000; // Above Unicode: an index into `clusters`.
const WIDE_TAIL = 0; // The second cell of a two-cell character.
// Kitty Unicode placeholder: this, then a row mark and a column mark, per cell.
// The foreground carries the image ID and the underline colour its placement.
const IMAGE_MARK = "\u{10eeee}";
const IMAGE_CELL = /\u{10eeee}(.)(.)/gu;

const els = {
  screen: document.getElementById("screen"),
  canvas: document.getElementById("grid"),
  cursor: document.getElementById("cursor"),
  selection: document.getElementById("selection"),
  input: document.getElementById("input"),
  banner: document.getElementById("banner"),
  bannerText: document.getElementById("banner-text"),
  bannerAction: document.getElementById("banner-action"),
};
const ctx = els.canvas.getContext("2d", { alpha: false });

function buildPalette() {
  const base = [
    0x1d2027, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
    0x5c6370, 0xf07f88, 0xa9d48b, 0xf0cf8e, 0x7cc0ff, 0xd694ea, 0x6dc9d6, 0xe6e9ef,
  ];
  const palette = base.slice();
  const levels = [0, 95, 135, 175, 215, 255];
  for (const r of levels) for (const g of levels) for (const b of levels) {
    palette.push((r << 16) | (g << 8) | b);
  }
  for (let i = 0; i < 24; i++) {
    const v = 8 + i * 10;
    palette.push((v << 16) | (v << 8) | v);
  }
  return palette;
}
const PALETTE = buildPalette();

// ---------------------------------------------------------------------------
// Cell grid
// ---------------------------------------------------------------------------

const grid = {
  cols: 0,
  rows: 0,
  cp: new Uint32Array(0),
  fg: new Int32Array(0),
  bg: new Int32Array(0),
  ul: new Int32Array(0),
  attr: new Uint8Array(0),
  ext: new Uint32Array(0), // Image cells: (image row << 16) | image column.
  dirty: new Uint8Array(0),
};
let clusters = [];
let clusterIndex = new Map();

function resizeGrid(cols, rows) {
  const old = { ...grid };
  grid.cols = cols;
  grid.rows = rows;
  const size = cols * rows;
  grid.cp = new Uint32Array(size).fill(32);
  grid.fg = new Int32Array(size).fill(NO_COLOR);
  grid.bg = new Int32Array(size).fill(NO_COLOR);
  grid.ul = new Int32Array(size).fill(NO_COLOR);
  grid.attr = new Uint8Array(size);
  grid.ext = new Uint32Array(size);
  grid.dirty = new Uint8Array(rows).fill(1);
  const keepCols = Math.min(cols, old.cols), keepRows = Math.min(rows, old.rows);
  for (let y = 0; y < keepRows; y++) {
    const from = y * old.cols, to = y * cols, end = from + keepCols;
    grid.cp.set(old.cp.subarray(from, end), to);
    grid.fg.set(old.fg.subarray(from, end), to);
    grid.bg.set(old.bg.subarray(from, end), to);
    grid.ul.set(old.ul.subarray(from, end), to);
    grid.attr.set(old.attr.subarray(from, end), to);
    grid.ext.set(old.ext.subarray(from, end), to);
  }
  vt.x = Math.min(vt.x, Math.max(cols - 1, 0));
  vt.y = Math.min(vt.y, Math.max(rows - 1, 0));
  vt.scrollBottom = rows - 1;
}

function clearCells(start, end, bg) {
  grid.cp.fill(32, start, end);
  grid.fg.fill(NO_COLOR, start, end);
  grid.bg.fill(bg, start, end);
  grid.ul.fill(NO_COLOR, start, end);
  grid.attr.fill(0, start, end);
}

function moveRows(from, to, count) {
  const cols = grid.cols;
  for (const array of [grid.cp, grid.fg, grid.bg, grid.ul, grid.attr, grid.ext]) {
    array.copyWithin(to * cols, from * cols, (from + count) * cols);
  }
}

// ---------------------------------------------------------------------------
// Character widths. The table comes from the server so a cell here is exactly
// a cell to the renderer that counted it.
// ---------------------------------------------------------------------------

let widthStarts = new Uint32Array(0), widthEnds = new Uint32Array(0), widthValues = new Uint8Array(0);
let diacriticIndex = new Map();
const segmenter = typeof Intl.Segmenter === "function"
  ? new Intl.Segmenter(undefined, { granularity: "grapheme" })
  : null;

function charWidth(cp) {
  if (cp < 0x80) return 1;
  let lo = 0, hi = widthStarts.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (cp < widthStarts[mid]) hi = mid - 1;
    else if (cp > widthEnds[mid]) lo = mid + 1;
    else return widthValues[mid];
  }
  return 1;
}

// Mirrors how a grapheme measures in `unicode-width`: a sequence that forms one
// emoji is two cells however many code points it took to spell it.
function clusterWidth(text) {
  const cps = Array.from(text, (c) => c.codePointAt(0));
  if (cps.length === 1) return charWidth(cps[0]);
  if (cps.includes(0x200d) || cps.includes(0xfe0f)) return 2;
  if (cps.length === 2 && cps[0] >= 0x1f1e6 && cps[0] <= 0x1f1ff) return 2;
  let width = 0;
  for (const cp of cps) width += charWidth(cp);
  return width;
}

function clusterCode(text) {
  let index = clusterIndex.get(text);
  if (index === undefined) {
    index = clusters.length;
    clusters.push(text);
    clusterIndex.set(text, index);
  }
  return CLUSTER_BASE + index;
}

// ---------------------------------------------------------------------------
// Terminal state. Only what `render.rs` emits is understood; anything else is
// skipped rather than guessed at.
// ---------------------------------------------------------------------------

const vt = {
  x: 0, y: 0,
  fg: NO_COLOR, bg: NO_COLOR, ul: NO_COLOR, attr: 0,
  scrollTop: 0, scrollBottom: 0,
  cursorVisible: false,
  cursorBar: false,
};

function putCell(cp, width, ext) {
  if (vt.y >= grid.rows || vt.x >= grid.cols) { vt.x += width; return; }
  const row = vt.y * grid.cols;
  const i = row + vt.x;
  // Overwriting half of a wide character blanks the other half.
  if (grid.cp[i] === WIDE_TAIL && vt.x > 0) grid.cp[i - 1] = 32;
  if (vt.x + 1 < grid.cols && grid.cp[i + 1] === WIDE_TAIL && width < 2) grid.cp[i + 1] = 32;
  grid.cp[i] = cp;
  grid.fg[i] = vt.fg;
  grid.bg[i] = vt.bg;
  grid.ul[i] = vt.ul;
  grid.attr[i] = vt.attr | (ext === undefined ? 0 : ATTR_IMAGE);
  grid.ext[i] = ext === undefined ? 0 : ext;
  if (width === 2 && vt.x + 1 < grid.cols) {
    const t = i + 1;
    grid.cp[t] = WIDE_TAIL;
    grid.fg[t] = vt.fg;
    grid.bg[t] = vt.bg;
    grid.ul[t] = vt.ul;
    grid.attr[t] = vt.attr;
    grid.ext[t] = 0;
  }
  grid.dirty[vt.y] = 1;
  vt.x += width;
}

function appendToPrevious(text) {
  if (vt.y >= grid.rows) return;
  let i = vt.y * grid.cols + Math.min(vt.x, grid.cols) - 1;
  if (i < vt.y * grid.cols) return;
  if (grid.cp[i] === WIDE_TAIL && i > vt.y * grid.cols) i--;
  grid.cp[i] = clusterCode(cellText(grid.cp[i]) + text);
  grid.dirty[vt.y] = 1;
}

function cellText(cp) {
  if (cp >= CLUSTER_BASE) return clusters[cp - CLUSTER_BASE];
  return String.fromCodePoint(cp || 32);
}

function printRun(text, start, end) {
  // The common case by far: a run of ASCII.
  let i = start;
  while (i < end) {
    const code = text.charCodeAt(i);
    if (code < 0x80) {
      if (code >= 0x20 && code !== 0x7f) putCell(code, 1);
      i++;
      continue;
    }
    let stop = i;
    while (stop < end && text.charCodeAt(stop) >= 0x80) stop++;
    printUnicode(text.slice(i, stop));
    i = stop;
  }
}

function printUnicode(run) {
  if (run.indexOf(IMAGE_MARK) < 0) { printGraphemes(run); return; }
  let at = 0;
  for (const match of run.matchAll(IMAGE_CELL)) {
    if (match.index > at) printGraphemes(run.slice(at, match.index));
    const row = diacriticIndex.get(match[1].codePointAt(0)) ?? 0;
    const col = diacriticIndex.get(match[2].codePointAt(0)) ?? 0;
    putCell(32, 1, (row << 16) | col);
    at = match.index + match[0].length;
  }
  if (at < run.length) printGraphemes(run.slice(at));
}

function printGraphemes(run) {
  const graphemes = segmenter ? segmenter.segment(run) : Array.from(run, (c) => ({ segment: c }));
  for (const { segment } of graphemes) {
    const width = clusterWidth(segment);
    if (width === 0) {
      appendToPrevious(segment);
      continue;
    }
    const cp = segment.codePointAt(0);
    putCell(segment.length === (cp > 0xffff ? 2 : 1) ? cp : clusterCode(segment), width);
  }
}

function sgr(params) {
  if (params.length === 0) params = [0];
  for (let i = 0; i < params.length; i++) {
    const p = params[i];
    switch (p) {
      case 0: vt.fg = vt.bg = vt.ul = NO_COLOR; vt.attr = 0; break;
      case 1: vt.attr |= ATTR_BOLD; break;
      case 2: vt.attr |= ATTR_DIM; break;
      case 3: vt.attr |= ATTR_ITALIC; break;
      case 4: vt.attr |= ATTR_UNDERLINE; break;
      case 7: vt.attr |= ATTR_REVERSE; break;
      case 9: vt.attr |= ATTR_STRIKE; break;
      case 21: case 22: vt.attr &= ~(ATTR_BOLD | ATTR_DIM); break;
      case 23: vt.attr &= ~ATTR_ITALIC; break;
      case 24: vt.attr &= ~ATTR_UNDERLINE; break;
      case 27: vt.attr &= ~ATTR_REVERSE; break;
      case 29: vt.attr &= ~ATTR_STRIKE; break;
      case 39: vt.fg = NO_COLOR; break;
      case 49: vt.bg = NO_COLOR; break;
      case 59: vt.ul = NO_COLOR; break;
      case 38: case 48: case 58: {
        let value = NO_COLOR;
        if (params[i + 1] === 5) {
          value = PALETTE[params[i + 2] & 255];
          i += 2;
        } else if (params[i + 1] === 2) {
          value = ((params[i + 2] & 255) << 16) | ((params[i + 3] & 255) << 8) | (params[i + 4] & 255);
          i += 4;
        } else {
          i = params.length;
        }
        if (p === 38) vt.fg = value; else if (p === 48) vt.bg = value; else vt.ul = value;
        break;
      }
      default:
        if (p >= 30 && p <= 37) vt.fg = PALETTE[p - 30];
        else if (p >= 40 && p <= 47) vt.bg = PALETTE[p - 40];
        else if (p >= 90 && p <= 97) vt.fg = PALETTE[p - 90 + 8];
        else if (p >= 100 && p <= 107) vt.bg = PALETTE[p - 100 + 8];
    }
  }
}

function scrollRegion(lines) {
  // Positive: delete lines at the top of the region (content moves up).
  const top = vt.scrollTop, bottom = vt.scrollBottom;
  const height = bottom - top + 1;
  if (height <= 0) return;
  const amount = Math.min(Math.abs(lines), height);
  const keep = height - amount;
  if (lines > 0) {
    moveRows(top + amount, top, keep);
    clearCells((top + keep) * grid.cols, (bottom + 1) * grid.cols, NO_COLOR);
  } else {
    moveRows(top, top + amount, keep);
    clearCells(top * grid.cols, (top + amount) * grid.cols, NO_COLOR);
  }
  grid.dirty.fill(1, top, bottom + 1);
}

function csi(final, intermediate, params) {
  const p0 = params.length ? params[0] : 0;
  switch (final) {
    case "H": case "f":
      vt.y = Math.max((params[0] || 1) - 1, 0);
      vt.x = Math.max((params[1] || 1) - 1, 0);
      break;
    case "G": vt.x = Math.max((p0 || 1) - 1, 0); break;
    case "m": if (intermediate === "") sgr(params); break;
    case "K": {
      if (vt.y >= grid.rows) break;
      const row = vt.y * grid.cols;
      const x = Math.min(vt.x, grid.cols);
      const [from, to] = p0 === 1 ? [row, row + x + 1] : p0 === 2 ? [row, row + grid.cols] : [row + x, row + grid.cols];
      clearCells(from, Math.min(to, row + grid.cols), vt.bg);
      grid.dirty[vt.y] = 1;
      if (vt.y === 0) setPageBackground(vt.bg);
      break;
    }
    case "J":
      if (p0 === 2 || p0 === 3) {
        clearCells(0, grid.cp.length, vt.bg);
        grid.dirty.fill(1);
      }
      break;
    case "r":
      vt.scrollTop = Math.max((params[0] || 1) - 1, 0);
      vt.scrollBottom = Math.min((params[1] || grid.rows) - 1, grid.rows - 1);
      vt.x = vt.y = 0;
      break;
    case "M": scrollRegion(p0 || 1); break;
    case "L": scrollRegion(-(p0 || 1)); break;
    case "S": scrollRegion(p0 || 1); break;
    case "T": scrollRegion(-(p0 || 1)); break;
    case "q":
      if (intermediate === " ") vt.cursorBar = p0 === 5 || p0 === 6;
      break;
    case "h": if (intermediate === "?" && p0 === 25) vt.cursorVisible = true; break;
    case "l": if (intermediate === "?" && p0 === 25) vt.cursorVisible = false; break;
  }
}

// Applies one frame. Everything the renderer flushes is complete sequences.
function applyFrame(text) {
  const n = text.length;
  let i = 0, runStart = -1;
  while (i < n) {
    const code = text.charCodeAt(i);
    if (code !== 0x1b) {
      if (runStart < 0) runStart = i;
      i++;
      continue;
    }
    if (runStart >= 0) { printRun(text, runStart, i); runStart = -1; }
    const kind = text.charCodeAt(i + 1);
    if (kind === 0x5b) { // CSI
      let j = i + 2, intermediate = "", value = -1;
      const params = [];
      for (; j < n; j++) {
        const c = text.charCodeAt(j);
        if (c >= 0x30 && c <= 0x39) value = (value < 0 ? 0 : value) * 10 + (c - 0x30);
        else if (c === 0x3b || c === 0x3a) { params.push(value < 0 ? 0 : value); value = -1; }
        else if (c === 0x3f || c === 0x3e || c === 0x3d || c === 0x3c || (c >= 0x20 && c <= 0x2f)) {
          intermediate += text[j];
        } else break;
      }
      if (value >= 0) params.push(value);
      if (j < n) csi(text[j], intermediate, params);
      i = j + 1;
    } else if (kind === 0x5f) { // APC: Kitty graphics
      const end = text.indexOf("\x1b\\", i + 2);
      const stop = end < 0 ? n : end;
      kittyCommand(text.slice(i + 2, stop));
      i = stop + 2;
    } else if (kind === 0x5d || kind === 0x50) { // OSC / DCS: nothing we use
      let end = text.indexOf("\x07", i + 2);
      const st = text.indexOf("\x1b\\", i + 2);
      if (end < 0 || (st >= 0 && st < end)) end = st < 0 ? n : st + 1;
      i = end + 1;
    } else {
      i += 2;
    }
  }
  if (runStart >= 0) printRun(text, runStart, n);
  schedulePaint();
}

// ---------------------------------------------------------------------------
// Kitty graphics: only the upload and delete commands the renderer sends.
// Placement geometry already travels in each placeholder cell's colours.
// ---------------------------------------------------------------------------

const images = new Map(); // id -> ImageBitmap
let upload = null; // { id, parts }
let imageEpoch = 0;

function kittyCommand(body) {
  if (body[0] !== "G") return;
  const semicolon = body.indexOf(";");
  const head = semicolon < 0 ? body.slice(1) : body.slice(1, semicolon);
  const payload = semicolon < 0 ? "" : body.slice(semicolon + 1);
  const keys = {};
  for (const pair of head.split(",")) {
    const [k, v] = pair.split("=");
    keys[k] = v;
  }
  if (keys.a === "t") {
    upload = { id: Number(keys.i), parts: [payload], epoch: imageEpoch };
    if (keys.m !== "1") finishUpload();
  } else if (keys.a === undefined && upload) {
    upload.parts.push(payload);
    if (keys.m !== "1") finishUpload();
  } else if (keys.a === "d" && keys.d === "I") {
    const id = Number(keys.i);
    images.get(id)?.close();
    images.delete(id);
  }
}

function finishUpload() {
  const { id, parts, epoch } = upload;
  upload = null;
  const binary = atob(parts.join(""));
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  createImageBitmap(new Blob([bytes], { type: "image/png" })).then((bitmap) => {
    if (epoch !== imageEpoch) { bitmap.close(); return; }
    images.get(id)?.close();
    images.set(id, bitmap);
    grid.dirty.fill(1);
    schedulePaint();
  }).catch(() => {});
}

function resetImages() {
  imageEpoch++;
  for (const bitmap of images.values()) bitmap.close();
  images.clear();
  upload = null;
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

const metrics = { dpr: 1, cellW: 8, cellH: 16, baseline: 12, spacing: 0, css: { w: 8, h: 16 } };
let fontSize = 14;
let fontFamily = FONT_STACK;
let hasLetterSpacing = false;
let nerdFont = true;
let paintPending = false;
let pageBackground = DEFAULT_BG;
// Read by the browser regression suite and the latency benchmark.
const stats = { frames: 0, paints: 0, bytes: 0, applyMs: 0, paintMs: 0, keyAt: 0, keyToFrameMs: 0, keyToPaintMs: 0 };
window.__vaayu = { stats, grid, vt, rowText, metrics, images };

function css(color) {
  return "#" + (color & 0xffffff).toString(16).padStart(6, "0");
}

function fontFor(attr) {
  return (attr & ATTR_ITALIC ? "italic " : "") + (attr & ATTR_BOLD ? "bold " : "") +
    (fontSize * metrics.dpr) + "px " + fontFamily;
}

function measureFont() {
  metrics.dpr = window.devicePixelRatio || 1;
  ctx.font = fontFor(0);
  const m = ctx.measureText("M".repeat(32));
  const natural = m.width / 32;
  const ascent = m.fontBoundingBoxAscent ?? fontSize * metrics.dpr * 0.8;
  const descent = m.fontBoundingBoxDescent ?? fontSize * metrics.dpr * 0.2;
  // Whole device pixels: a fractional cell would leave seams between runs.
  metrics.cellW = Math.max(1, Math.round(natural));
  metrics.cellH = Math.max(1, Math.ceil((ascent + descent) * 1.12));
  metrics.baseline = Math.round((metrics.cellH + ascent - descent) / 2);
  metrics.spacing = metrics.cellW - natural;
  metrics.css = { w: metrics.cellW / metrics.dpr, h: metrics.cellH / metrics.dpr };
  hasLetterSpacing = "letterSpacing" in ctx;
  detectNerdFont();
}

// Tree and picker icons are Nerd Font glyphs. Without such a font a browser
// draws boxes, so those are painted as simple shapes in the icon's colour.
function detectNerdFont() {
  const probe = document.createElement("canvas");
  probe.width = probe.height = 32;
  const c = probe.getContext("2d", { willReadFrequently: true });
  const inkOf = (text) => {
    c.clearRect(0, 0, 32, 32);
    c.font = "20px " + fontFamily;
    c.fillStyle = "#000";
    c.fillText(text, 2, 24);
    return c.getImageData(0, 0, 32, 32).data.join(",");
  };
  const folder = inkOf("");
  nerdFont = folder !== inkOf("0") && folder !== inkOf("");
}

function layout() {
  measureFont();
  const cols = Math.max(2, Math.floor(window.innerWidth / metrics.css.w));
  const rows = Math.max(2, Math.floor(window.innerHeight / metrics.css.h));
  const width = cols * metrics.cellW, height = rows * metrics.cellH;
  els.canvas.width = width;
  els.canvas.height = height;
  els.canvas.style.width = width / metrics.dpr + "px";
  els.canvas.style.height = height / metrics.dpr + "px";
  // After a canvas resize the context state is back to its defaults.
  ctx.textBaseline = "alphabetic";
  if ("fontKerning" in ctx) ctx.fontKerning = "none";
  if ("textRendering" in ctx) ctx.textRendering = "optimizeSpeed";
  lastFont = "";
  lastSpacing = "";
  const changed = cols !== grid.cols || rows !== grid.rows;
  if (changed) resizeGrid(cols, rows);
  grid.dirty.fill(1);
  schedulePaint();
  return { cols, rows, changed };
}

let lastFont = "", lastSpacing = "";
function setFont(font, spacing) {
  if (font !== lastFont) { ctx.font = font; lastFont = font; }
  if (hasLetterSpacing && spacing !== lastSpacing) { ctx.letterSpacing = spacing; lastSpacing = spacing; }
}

function foreground(i) {
  if (grid.attr[i] & ATTR_REVERSE) {
    const bg = grid.bg[i];
    return bg === NO_COLOR ? pageBackground : bg;
  }
  const fg = grid.fg[i];
  return fg === NO_COLOR ? DEFAULT_FG : fg;
}

function background(i) {
  if (grid.attr[i] & ATTR_REVERSE) {
    const fg = grid.fg[i];
    return fg === NO_COLOR ? DEFAULT_FG : fg;
  }
  const bg = grid.bg[i];
  return bg === NO_COLOR ? pageBackground : bg;
}

function dimmed(color, bg) {
  const mix = (a, b) => Math.round(a * 0.6 + b * 0.4);
  return (mix(color >> 16 & 255, bg >> 16 & 255) << 16) |
    (mix(color >> 8 & 255, bg >> 8 & 255) << 8) |
    mix(color & 255, bg & 255);
}

// Line drawing is painted, not typeset, so borders join with no font-dependent
// gaps. Each entry gives the weight of the arm going up, right, down and left.
const BOX = new Map(Object.entries({
  "─": [0, 1, 0, 1], "│": [1, 0, 1, 0], "┌": [0, 1, 1, 0], "┐": [0, 0, 1, 1],
  "└": [1, 1, 0, 0], "┘": [1, 0, 0, 1], "├": [1, 1, 1, 0], "┤": [1, 0, 1, 1],
  "┬": [0, 1, 1, 1], "┴": [1, 1, 0, 1], "┼": [1, 1, 1, 1], "╭": [0, 1, 1, 0],
  "╮": [0, 0, 1, 1], "╯": [1, 0, 0, 1], "╰": [1, 1, 0, 0], "━": [0, 2, 0, 2],
  "┃": [2, 0, 2, 0], "┏": [0, 2, 2, 0], "┓": [0, 0, 2, 2], "┗": [2, 2, 0, 0],
  "┛": [2, 0, 0, 2], "┣": [2, 2, 2, 0], "┫": [2, 0, 2, 2], "┳": [0, 2, 2, 2],
  "┻": [2, 2, 0, 2], "╋": [2, 2, 2, 2], "╴": [0, 0, 0, 1], "╵": [1, 0, 0, 0],
  "╶": [0, 1, 0, 0], "╷": [0, 0, 1, 0],
}).map(([k, v]) => [k.codePointAt(0), v]));

function drawBox(arms, x, y, w, h) {
  const thin = Math.max(1, Math.round(metrics.dpr));
  const cx = x + ((w - thin) >> 1), cy = y + ((h - thin) >> 1);
  const size = (weight) => (weight === 2 ? thin * 2 : thin);
  const [up, right, down, left] = arms;
  if (up) ctx.fillRect(cx, y, size(up), cy - y + thin);
  if (down) ctx.fillRect(cx, cy, size(down), y + h - cy);
  if (left) ctx.fillRect(x, cy, cx - x + thin, size(left));
  if (right) ctx.fillRect(cx, cy, x + w - cx, size(right));
}

function drawBlock(cp, x, y, w, h) {
  if (cp === 0x2588) { ctx.fillRect(x, y, w, h); return true; }
  if (cp === 0x2580) { ctx.fillRect(x, y, w, h >> 1); return true; }
  if (cp >= 0x2581 && cp <= 0x2587) { // Lower eighths.
    const part = Math.round(h * (cp - 0x2580) / 8);
    ctx.fillRect(x, y + h - part, w, part);
    return true;
  }
  if (cp >= 0x2589 && cp <= 0x258f) { // Left eighths.
    ctx.fillRect(x, y, Math.max(1, Math.round(w * (0x2590 - cp) / 8)), h);
    return true;
  }
  if (cp === 0x2590) { ctx.fillRect(x + (w >> 1), y, w - (w >> 1), h); return true; }
  if (cp === 0x2594) { ctx.fillRect(x, y, w, Math.max(1, h >> 3)); return true; }
  if (cp === 0x2595) { const t = Math.max(1, w >> 3); ctx.fillRect(x + w - t, y, t, h); return true; }
  if (cp >= 0x2591 && cp <= 0x2593) {
    ctx.save();
    ctx.globalAlpha = (cp - 0x2590) * 0.25;
    ctx.fillRect(x, y, w, h);
    ctx.restore();
    return true;
  }
  return false;
}

function drawIconFallback(cp, x, y, w, h) {
  const s = Math.max(4, Math.min(w, h) - Math.round(metrics.dpr * 2));
  const left = x + ((w - s) >> 1), top = y + ((h - s) >> 1);
  if (cp === 0xf07b || cp === 0xf07c || cp === 0xf482) { // Folder.
    ctx.fillRect(left, top + (s >> 3), s >> 1, s >> 3);
    ctx.fillRect(left, top + (s >> 3) * 2, s, s - (s >> 2));
  } else {
    const dot = Math.max(3, s >> 1);
    ctx.fillRect(x + ((w - dot) >> 1), y + ((h - dot) >> 1), dot, dot);
  }
}

function paintRow(y) {
  const { cols } = grid;
  const { cellW: w, cellH: h, baseline } = metrics;
  const start = y * cols;
  const top = y * h;

  // Backgrounds, as runs.
  let x = 0;
  while (x < cols) {
    const bg = background(start + x);
    let end = x + 1;
    while (end < cols && background(start + end) === bg) end++;
    ctx.fillStyle = css(bg);
    ctx.fillRect(x * w, top, (end - x) * w, h);
    x = end;
  }

  // Glyphs. A row is clipped so an overhanging italic can't dirty its neighbour.
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, top, cols * w, h);
  ctx.clip();
  const text = top + baseline;
  x = 0;
  while (x < cols) {
    const i = start + x;
    const cp = grid.cp[i];
    const attr = grid.attr[i];
    const fg = foreground(i);
    const color = css(attr & ATTR_DIM ? dimmed(fg, background(i)) : fg);

    if (attr & ATTR_IMAGE) {
      const id = grid.fg[i] & 0xffffff;
      const placement = grid.ul[i] & 0xffffff;
      const bitmap = images.get(id);
      const columns = placement >> 9, rows = placement & 0x1ff;
      if (bitmap && columns > 0 && rows > 0) {
        const row = grid.ext[i] >>> 16, col = grid.ext[i] & 0xffff;
        const sw = bitmap.width / columns, sh = bitmap.height / rows;
        ctx.drawImage(bitmap, col * sw, row * sh, sw, sh, x * w, top, w, h);
      }
      x++;
      continue;
    }

    if (cp === 32 || cp === WIDE_TAIL) {
      if (attr & (ATTR_UNDERLINE | ATTR_STRIKE)) decorate(attr, grid.ul[i], color, x * w, top, w, h);
      x++;
      continue;
    }

    ctx.fillStyle = color;
    const arms = BOX.get(cp);
    if (arms) {
      drawBox(arms, x * w, top, w, h);
      x++;
      continue;
    }
    if (cp >= 0x2580 && cp <= 0x259f && drawBlock(cp, x * w, top, w, h)) {
      x++;
      continue;
    }
    if (!nerdFont && cp >= 0xe000 && cp <= 0xf8ff) {
      drawIconFallback(cp, x * w, top, w, h);
      x++;
      continue;
    }

    // A run of ASCII in one style goes to the canvas as one string.
    if (cp < 0x7f) {
      const fgRaw = grid.fg[i], bgRaw = grid.bg[i], ulRaw = grid.ul[i];
      let end = x + 1;
      let run = String.fromCharCode(cp);
      while (end < cols) {
        const k = start + end;
        const next = grid.cp[k];
        if (next >= 0x7f || next === WIDE_TAIL || grid.attr[k] !== attr ||
            grid.fg[k] !== fgRaw || grid.bg[k] !== bgRaw || grid.ul[k] !== ulRaw) break;
        run += String.fromCharCode(next);
        end++;
      }
      if (hasLetterSpacing) {
        setFont(fontFor(attr), metrics.spacing + "px");
        ctx.fillText(run, x * w, text);
      } else {
        setFont(fontFor(attr), "");
        for (let k = 0; k < run.length; k++) {
          if (run.charCodeAt(k) !== 32) ctx.fillText(run[k], (x + k) * w, text);
        }
      }
      if (attr & (ATTR_UNDERLINE | ATTR_STRIKE)) decorate(attr, ulRaw, color, x * w, top, (end - x) * w, h);
      x = end;
      continue;
    }

    setFont(fontFor(attr), "0px");
    ctx.fillText(cellText(cp), x * w, text);
    const tail = x + 1 < cols && grid.cp[i + 1] === WIDE_TAIL ? 2 : 1;
    if (attr & (ATTR_UNDERLINE | ATTR_STRIKE)) decorate(attr, grid.ul[i], color, x * w, top, tail * w, h);
    x += tail;
  }
  ctx.restore();
  // restore() also rewinds the font and spacing behind setFont's back.
  lastFont = "";
  lastSpacing = "";
}

function decorate(attr, underline, color, x, y, width, h) {
  const thickness = Math.max(1, Math.round(metrics.dpr));
  if (attr & ATTR_UNDERLINE) {
    ctx.fillStyle = underline === NO_COLOR ? color : css(underline);
    ctx.fillRect(x, y + metrics.baseline + thickness, width, thickness);
  }
  if (attr & ATTR_STRIKE) {
    ctx.fillStyle = color;
    ctx.fillRect(x, y + (h >> 1), width, thickness);
  }
}

function schedulePaint() {
  if (paintPending) return;
  paintPending = true;
  requestAnimationFrame(paint);
}

function paint() {
  paintPending = false;
  const started = performance.now();
  for (let y = 0; y < grid.rows; y++) {
    if (grid.dirty[y]) {
      grid.dirty[y] = 0;
      paintRow(y);
    }
  }
  placeCursor();
  const finished = performance.now();
  stats.paints++;
  stats.paintMs += finished - started;
  if (stats.keyAt) {
    stats.keyToPaintMs = finished - stats.keyAt;
    stats.keyAt = 0;
  }
}

function placeCursor() {
  const { css: cell } = metrics;
  const visible = vt.cursorVisible && vt.x < grid.cols && vt.y < grid.rows;
  els.cursor.hidden = !visible;
  const left = Math.min(vt.x, grid.cols - 1) * cell.w;
  const top = Math.min(vt.y, grid.rows - 1) * cell.h;
  if (visible) {
    const wide = vt.x + 1 < grid.cols && grid.cp[vt.y * grid.cols + vt.x + 1] === WIDE_TAIL;
    const width = vt.cursorBar ? Math.max(2 / metrics.dpr, cell.w / 8) : cell.w * (wide ? 2 : 1);
    els.cursor.style.width = width + "px";
    els.cursor.style.height = cell.h + "px";
    els.cursor.style.transform = `translate(${left}px, ${top}px)`;
  }
  els.input.style.transform = `translate(${left}px, ${top}px)`;
}

function setPageBackground(bg) {
  const color = bg === NO_COLOR ? DEFAULT_BG : bg;
  if (color === pageBackground) return;
  pageBackground = color;
  document.documentElement.style.setProperty("--page-bg", css(color));
  // Unset cells paint with this colour too.
  grid.dirty.fill(1);
}

function rowText(y) {
  let out = "";
  for (let x = 0; x < grid.cols; x++) {
    const cp = grid.cp[y * grid.cols + x];
    if (cp !== WIDE_TAIL) out += cellText(cp);
  }
  return out;
}

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

let socket = null;
let attachedOnce = false;
let sessionOver = false;
let superseded = false;
let retryDelay = 200;
const decoder = new TextDecoder();

function send(message) {
  if (socket && socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify(message));
}

function sendSize() {
  send({ t: "size", cols: grid.cols, rows: grid.rows, cw: Math.round(metrics.css.w), ch: Math.round(metrics.css.h) });
}

function showBanner(text, actionLabel, action) {
  els.banner.hidden = false;
  els.bannerText.textContent = text;
  els.bannerAction.hidden = !actionLabel;
  els.bannerAction.textContent = actionLabel || "";
  els.bannerAction.onclick = action || null;
  if (actionLabel) els.bannerAction.focus();
}

function hideBanner() {
  els.banner.hidden = true;
}

function connect() {
  superseded = false;
  const current = new WebSocket(`ws://${location.host}/ws`);
  socket = current;
  current.binaryType = "arraybuffer";
  current.onopen = () => { retryDelay = 200; };
  current.onmessage = (event) => {
    if (typeof event.data === "string") controlMessage(JSON.parse(event.data));
    else frameMessage(event.data);
  };
  current.onclose = () => {
    if (current !== socket || sessionOver || superseded) return;
    // A failed handshake and a stopped server look alike from here, so the
    // first drop retries silently; only a persistent one says anything.
    if (attachedOnce) showBanner("Lost the connection to the editor. Reconnecting…");
    setTimeout(connect, retryDelay);
    retryDelay = Math.min(retryDelay * 2, 3000);
    if (retryDelay >= 3000) checkSessionGone();
  };
}

async function checkSessionGone() {
  try {
    const response = await fetch("/", { cache: "no-store" });
    if (response.status === 401) {
      sessionOver = true;
      showBanner("This session has been replaced. Run `vaayu --web` in the project for a new link.");
    }
  } catch {
    showBanner("The editor is not responding. Reconnecting…\nIf it has stopped, run `vaayu --web` in the project again.");
  }
}

function controlMessage(message) {
  switch (message.t) {
    case "hello":
      configure(message);
      resetDisplay();
      attachedOnce = true;
      hideBanner();
      sendSize();
      send({ t: "focus" });
      els.input.focus({ preventScroll: true });
      syncClipboardMirror();
      break;
    case "title":
      document.title = message.title;
      break;
    case "clipboard":
      writeClipboard(message.text);
      break;
    case "superseded":
      superseded = true;
      showBanner("This session is now open in another tab.", "Use it here", () => {
        hideBanner();
        connect();
      });
      break;
    case "bye":
      sessionOver = true;
      window.removeEventListener("beforeunload", guardUnload);
      document.title = "Vaayu — session ended";
      els.cursor.hidden = true;
      showBanner("The editor has quit. You can close this tab.");
      break;
  }
}

function configure(hello) {
  const table = hello.widths;
  widthStarts = new Uint32Array(table.length);
  widthEnds = new Uint32Array(table.length);
  widthValues = new Uint8Array(table.length);
  table.forEach(([first, last, width], i) => {
    widthStarts[i] = first;
    widthEnds[i] = last;
    widthValues[i] = width;
  });
  diacriticIndex = new Map(hello.diacritics.map((cp, i) => [cp, i]));
  const family = hello.font.family.trim();
  fontFamily = family ? `${family}, ${FONT_STACK}` : FONT_STACK;
  fontSize = hello.font.size || 14;
  layout();
}

function resetDisplay() {
  resetImages();
  clusters = [];
  clusterIndex = new Map();
  clearCells(0, grid.cp.length, NO_COLOR);
  grid.ext.fill(0);
  grid.dirty.fill(1);
  Object.assign(vt, {
    x: 0, y: 0, fg: NO_COLOR, bg: NO_COLOR, ul: NO_COLOR, attr: 0,
    scrollTop: 0, scrollBottom: grid.rows - 1, cursorVisible: false, cursorBar: false,
  });
}

function frameMessage(buffer) {
  const started = performance.now();
  applyFrame(decoder.decode(buffer));
  stats.frames++;
  stats.bytes += buffer.byteLength;
  const finished = performance.now();
  stats.applyMs += finished - started;
  if (stats.keyAt) stats.keyToFrameMs = finished - stats.keyAt;
}

// ---------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------

async function writeClipboard(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Needs a focused page; the server's copy still has it for its own paste.
  }
}

// `"+p` is answered on the server, which can't wait for the page. When the
// browser allows reading without a prompt, keep it told what is on the clipboard.
async function syncClipboardMirror() {
  try {
    if (!navigator.clipboard?.readText || !navigator.permissions) return;
    const state = await navigator.permissions.query({ name: "clipboard-read" });
    if (state.state !== "granted") return;
    send({ t: "clipboard", text: await navigator.clipboard.readText() });
  } catch {
    // Firefox has no such permission; pasting still works through the browser.
  }
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

// Browsers keep these from a page entirely, and the editor binds their Ctrl
// forms (window commands, tabs, completion). Alt+letter stands in for them.
const ALT_FOR_CTRL = new Set(["w", "t", "n"]);

const NAMED_KEYS = new Set([
  "Enter", "Escape", "Backspace", "Tab", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown",
  "Home", "End", "Delete", "PageUp", "PageDown",
]);

// What the browser keeps: its zoom, developer tools, reload and fullscreen.
function belongsToBrowser(e) {
  if (e.metaKey) return true;
  if (e.key === "F5" || e.key === "F12") return true;
  if (e.ctrlKey && !e.altKey && ["=", "+", "-", "0"].includes(e.key)) return true;
  if (e.ctrlKey && e.shiftKey && ["I", "J", "C", "R", "V"].includes(e.key.toUpperCase())) return true;
  return false;
}

function isAltGraph(e) {
  return e.getModifierState && e.getModifierState("AltGraph");
}

function onKeyDown(e) {
  if (!els.banner.hidden || e.isComposing || e.keyCode === 229) return;
  if (belongsToBrowser(e)) {
    // Ctrl+Shift+C copies what was selected with Shift+drag.
    if (e.ctrlKey && e.shiftKey && e.key.toUpperCase() === "C" && selection.text) {
      writeClipboard(selection.text);
      clearSelection();
      e.preventDefault();
    }
    return;
  }

  if (e.key === "F11") {
    e.preventDefault();
    toggleFullscreen();
    return;
  }

  let key = e.key, ctrl = e.ctrlKey, alt = e.altKey;
  if (isAltGraph(e)) {
    ctrl = alt = false;
  } else if (alt && !ctrl && ALT_FOR_CTRL.has(key.toLowerCase())) {
    key = key.toLowerCase();
    ctrl = true;
    alt = false;
  }
  if (!NAMED_KEYS.has(key) && Array.from(key).length !== 1) return;

  e.preventDefault();
  clearSelection();
  stats.keyAt = performance.now();
  send({ t: "key", key, ctrl, alt, shift: e.shiftKey });
}

// In page-requested fullscreen Chromium can hand over even Ctrl-W and Ctrl-T, and
// leaving takes a long press of Esc, so Esc stays the editor's.
async function toggleFullscreen() {
  try {
    if (document.fullscreenElement) {
      await document.exitFullscreen();
    } else {
      await document.documentElement.requestFullscreen();
      await navigator.keyboard?.lock?.();
    }
  } catch {
    // Refused by the browser; the tab simply stays as it was.
  }
  els.input.focus({ preventScroll: true });
}

// Text arriving without a usable keydown: dead keys, IMEs, on-screen keyboards.
function flushInput() {
  const text = els.input.value;
  els.input.value = "";
  for (const c of text) send({ t: "key", key: c });
}

function onInput(e) {
  if (e.isComposing) return;
  flushInput();
}

function onCompositionStart() {
  els.input.classList.add("composing");
}

function onCompositionEnd() {
  els.input.classList.remove("composing");
  flushInput();
}

function onPaste(e) {
  const text = e.clipboardData?.getData("text") ?? "";
  e.preventDefault();
  if (!text) return;
  send({ t: "clipboard", text });
  send({ t: "paste", text });
}

// ---------------------------------------------------------------------------
// Mouse
// ---------------------------------------------------------------------------

const pointer = { down: false, button: 0, col: -1, row: -1 };
const selection = { active: false, start: null, end: null, text: "" };
let wheelY = 0, wheelX = 0;

function cellAt(e) {
  const rect = els.canvas.getBoundingClientRect();
  return {
    col: Math.max(0, Math.min(grid.cols - 1, Math.floor((e.clientX - rect.left) / metrics.css.w))),
    row: Math.max(0, Math.min(grid.rows - 1, Math.floor((e.clientY - rect.top) / metrics.css.h))),
  };
}

function sendMouse(kind, button, cell, e) {
  send({ t: "mouse", kind, button, col: cell.col, row: cell.row, ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey });
}

function onMouseDown(e) {
  e.preventDefault();
  els.input.focus({ preventScroll: true });
  const cell = cellAt(e);
  clearSelection();
  // As in a terminal, Shift+drag selects text in the display itself.
  if (e.shiftKey && e.button === 0) {
    selection.active = true;
    selection.start = selection.end = cell;
    return;
  }
  pointer.down = true;
  pointer.button = e.button;
  pointer.col = cell.col;
  pointer.row = cell.row;
  sendMouse("down", e.button, cell, e);
}

function onMouseMove(e) {
  const cell = cellAt(e);
  if (selection.active) {
    selection.end = cell;
    drawSelection();
    return;
  }
  if (cell.col === pointer.col && cell.row === pointer.row) return;
  pointer.col = cell.col;
  pointer.row = cell.row;
  sendMouse(pointer.down ? "drag" : "move", pointer.button, cell, e);
}

function onMouseUp(e) {
  if (selection.active) {
    selection.active = false;
    selection.text = selectedText();
    if (selection.text) writeClipboard(selection.text);
    return;
  }
  if (!pointer.down) return;
  pointer.down = false;
  sendMouse("up", e.button, cellAt(e), e);
}

function onWheel(e) {
  e.preventDefault();
  const unit = e.deltaMode === 1 ? metrics.css.h : e.deltaMode === 2 ? window.innerHeight : 1;
  wheelY += e.deltaY * unit;
  wheelX += e.deltaX * unit;
  const cell = cellAt(e);
  const step = metrics.css.h;
  while (Math.abs(wheelY) >= step) {
    sendMouse(wheelY > 0 ? "wheeldown" : "wheelup", 0, cell, e);
    wheelY -= Math.sign(wheelY) * step;
  }
  while (Math.abs(wheelX) >= step) {
    sendMouse(wheelX > 0 ? "wheelright" : "wheelleft", 0, cell, e);
    wheelX -= Math.sign(wheelX) * step;
  }
}

function orderedSelection() {
  const { start, end } = selection;
  if (!start || !end) return null;
  const forward = start.row < end.row || (start.row === end.row && start.col <= end.col);
  return forward ? [start, end] : [end, start];
}

function drawSelection() {
  els.selection.replaceChildren();
  const range = orderedSelection();
  if (!range) { els.selection.hidden = true; return; }
  const [from, to] = range;
  const { w, h } = metrics.css;
  for (let row = from.row; row <= to.row; row++) {
    const first = row === from.row ? from.col : 0;
    const last = row === to.row ? to.col : grid.cols - 1;
    const part = document.createElement("div");
    part.style.cssText = `left:${first * w}px;top:${row * h}px;width:${(last - first + 1) * w}px;height:${h}px`;
    els.selection.append(part);
  }
  els.selection.hidden = false;
}

function selectedText() {
  const range = orderedSelection();
  if (!range) return "";
  const [from, to] = range;
  if (from.row === to.row && from.col === to.col) return "";
  const lines = [];
  for (let row = from.row; row <= to.row; row++) {
    const first = row === from.row ? from.col : 0;
    const last = row === to.row ? to.col : grid.cols - 1;
    let line = "";
    for (let col = first; col <= last; col++) {
      const cp = grid.cp[row * grid.cols + col];
      if (cp !== WIDE_TAIL) line += cellText(cp);
    }
    lines.push(line.trimEnd());
  }
  return lines.join("\n");
}

function clearSelection() {
  selection.active = false;
  selection.start = selection.end = null;
  selection.text = "";
  els.selection.hidden = true;
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

function guardUnload(e) {
  // Ctrl-W is a window command in the editor and "close tab" to the browser.
  e.preventDefault();
  e.returnValue = "";
}

let resizeTimer = 0;
function onResize() {
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => {
    const { changed } = layout();
    if (changed) {
      clearSelection();
      sendSize();
    }
  }, 30);
}

function setFocused(focused) {
  els.cursor.classList.toggle("unfocused", !focused);
}

els.input.addEventListener("keydown", onKeyDown);
els.input.addEventListener("input", onInput);
els.input.addEventListener("compositionstart", onCompositionStart);
els.input.addEventListener("compositionend", onCompositionEnd);
els.input.addEventListener("paste", onPaste);
els.screen.addEventListener("mousedown", onMouseDown);
els.screen.addEventListener("contextmenu", (e) => e.preventDefault());
els.screen.addEventListener("wheel", onWheel, { passive: false });
window.addEventListener("mousemove", onMouseMove);
window.addEventListener("mouseup", onMouseUp);
window.addEventListener("resize", onResize);
window.addEventListener("beforeunload", guardUnload);
window.addEventListener("focus", () => {
  setFocused(true);
  els.input.focus({ preventScroll: true });
  send({ t: "focus" });
  syncClipboardMirror();
});
window.addEventListener("blur", () => setFocused(false));
// A newly loaded web font changes the cell metrics.
document.fonts?.addEventListener?.("loadingdone", onResize);

// Lets the benchmark ask for a repaint without a frame from the server.
window.addEventListener("vaayu-repaint", schedulePaint);

layout();
connect();
