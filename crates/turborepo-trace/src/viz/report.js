"use strict";
// The impact report. Reads the data embedded in the page and draws it; it requests nothing.
// Every string from the data goes into the page as text, never as markup.
(function () {
  const D = JSON.parse(document.getElementById("data").textContent);
  const SVGNS = document.getElementById("defs").namespaceURI;
  const $ = (id) => document.getElementById(id);
  const E = D.entries;
  // Entry fields.
  const P = 0, K = 1, G = 2, V = 3, C = 4, CM = 5, S = 6, X = 7, DOM = 8, CH = 9, Y = 10, O = 11, ON = 12, U = 13, H = 14, EW = 15;
  const NOHUB = -2, OTHER = -3;

  const nf = new Intl.NumberFormat("en-US");
  const num = (x) => nf.format(x);
  const pct = (a, b) => {
    if (!b || !a) return "0%";
    const p = (100 * a) / b;
    return p > 0 && p < 0.1 ? "<0.1%" : (p >= 99.95 && a < b ? "99.9" : p.toFixed(p < 10 ? 1 : 0)) + "%";
  };
  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined && text !== null) e.textContent = text;
    return e;
  }
  function add(parent, ...kids) {
    for (const k of kids) if (k !== null && k !== undefined) parent.append(k);
    return parent;
  }
  function svg(tag, attrs, text) {
    const e = document.createElementNS(SVGNS, tag);
    for (const k in attrs) e.setAttribute(k, attrs[k]);
    if (text !== undefined) e.textContent = text;
    return e;
  }
  const clear = (e) => { while (e.firstChild) e.removeChild(e.firstChild); return e; };

  // What the entries are called.
  const kinds = D.kinds.filter((k) => k.total > 0);
  const allTests = kinds.length > 0 && kinds.every((k) => k.id !== "task" && k.id !== "file");
  const allTasks = kinds.length > 0 && kinds.every((k) => k.id === "task");
  const noun = (n) => (allTests ? (n === 1 ? "test" : "tests") : allTasks ? (n === 1 ? "task" : "tasks") : n === 1 ? "entry" : "entries");
  const T = D.totals;
  const chosen = T.selected + T.unknown;

  // Paths: the directory is quiet, the name is not.
  function splitPath(p) {
    const i = p.lastIndexOf("/");
    return i < 0 ? ["", p] : [p.slice(0, i + 1), p.slice(i + 1)];
  }
  function pathNode(p) {
    const [dir, base] = splitPath(p);
    const b = el("bdi");
    if (dir) b.append(el("span", "dir", dir));
    b.append(base);
    return b;
  }
  const sourceText = (s) => (s.kind === "site" ? s.label : s.kind === "package" ? s.label : s.label);
  const isPathSource = (s) => s.kind === "change" || s.kind === "site";
  const CLASS = { source: "source file", test: "test file", mock: "mock", story: "story", dts: "type declarations", fixture: "fixture", manifest: "package.json", tsconfig: "tsconfig", "env-file": "env file", "tool-config": "tool config", lockfile: "lockfile", snapshot: "snapshot", asset: "asset", doc: "document", data: "data file", other: "file" };
  const sourceKind = (s) => ({ change: CLASS[s.class] || (s.class.startsWith("node:") ? "task" : s.class), site: "open uncertainty", package: "package version", name: "name in a string", layer: "rule", other: "other" })[s.kind] || s.kind;

  const lower = new Array(E.length);
  const low = (i) => lower[i] || (lower[i] = D.nodes[E[i][P]].toLowerCase());

  // State.
  const state = { q: "", causes: new Set(), source: -1, hub: -1, group: -1, verdict: "selected", open: -1 };
  let rows = []; // selected entries that pass every filter
  let base = []; // selected entries that pass the search and the causes only

  // The hub an entry is drawn through. With a hub chosen: the next hub that every path
  // crosses after it.
  const hubOf = (e) => {
    if (state.hub >= 0) {
      const i = e[DOM].indexOf(state.hub);
      return i >= 0 && i + 1 < e[DOM].length ? e[DOM][i + 1] : NOHUB;
    }
    return e[H] >= 0 ? e[H] : NOHUB;
  };
  const drilled = () => state.hub >= 0;
  const throughHub = (e, h) => (h === NOHUB ? e[H] < 0 : e[H] === h || e[DOM].includes(h));
  function matchesText(i, q) {
    const e = E[i];
    if (low(i).includes(q)) return true;
    if (e[S] >= 0 && D.sources[e[S]].label.toLowerCase().includes(q)) return true;
    if (e[H] >= 0 && D.nodes[D.hubs[e[H]].node].toLowerCase().includes(q)) return true;
    return false;
  }
  function compute() {
    const q = state.q.trim().toLowerCase();
    rows = [];
    base = [];
    for (let i = 0; i < E.length; i++) {
      const e = E[i];
      if (e[V] === 0) continue;
      if (q && !matchesText(i, q)) continue;
      if (state.causes.size && !state.causes.has(e[C])) continue;
      base.push(i);
      if (state.source >= 0 && e[S] !== state.source) continue;
      if (state.hub !== -1 && !throughHub(e, state.hub)) continue;
      if (state.group >= 0 && e[G] !== state.group) continue;
      rows.push(i);
    }
  }
  const filtered = () => state.q.trim() !== "" || state.causes.size > 0 || state.source >= 0 || state.hub !== -1 || state.group >= 0;

  // ---------------------------------------------------------------- tooltip
  const tip = $("tip");
  function showTip(build, x, y) {
    clear(tip);
    build(tip);
    tip.hidden = false;
    const r = tip.getBoundingClientRect();
    let left = x + 14, top = y + 16;
    if (left + r.width > window.innerWidth - 8) left = Math.max(8, x - r.width - 14);
    if (top + r.height > window.innerHeight - 8) top = Math.max(8, y - r.height - 12);
    tip.style.left = left + "px";
    tip.style.top = top + "px";
  }
  const hideTip = () => { tip.hidden = true; };
  // `keys`: also on keyboard focus. (An SVG element with a focus listener takes the focus,
  // so the ribbons, which the blocks stand for, get none.)
  function tipOn(target, build, keys) {
    target.addEventListener("pointermove", (ev) => showTip(build, ev.clientX, ev.clientY));
    target.addEventListener("pointerleave", hideTip);
    if (keys === false) return;
    target.addEventListener("focus", () => {
      const r = target.getBoundingClientRect();
      showTip(build, r.left + Math.min(r.width, 240) / 2, r.bottom - 8);
    });
    target.addEventListener("blur", hideTip);
  }

  // ---------------------------------------------------------------- text measuring
  const ctx = document.createElement("canvas").getContext("2d");
  const fonts = {};
  function fontOf(cls) {
    if (!fonts[cls]) {
      const probe = svg("text", { class: cls }, "x");
      const host = svg("svg", { width: 0, height: 0 });
      host.append(probe);
      const view = $("flow-view");
      view.append(host);
      const cs = getComputedStyle(probe);
      fonts[cls] = cs.fontWeight + " " + cs.fontSize + " " + cs.fontFamily;
      host.remove();
    }
    return fonts[cls];
  }
  function width(text, cls) {
    ctx.font = fontOf(cls);
    return ctx.measureText(text).width;
  }
  // A path cut on the left, directory by directory, to fit `max` pixels.
  function fitPath(p, max, cls) {
    let [dir, name] = splitPath(p);
    if (width(dir + name, cls) <= max) return [dir, name];
    const parts = dir.split("/").filter(Boolean);
    while (parts.length) {
      parts.shift();
      const d = "…/" + parts.join("/") + (parts.length ? "/" : "");
      if (width(d + name, cls) <= max) return [d, name];
    }
    while (name.length > 4 && width("…" + name, cls) > max) name = name.slice(1);
    return ["", "…" + name];
  }
  function fitText(s, max, cls) {
    if (width(s, cls) <= max) return s;
    while (s.length > 2 && width(s + "…", cls) > max) s = s.slice(0, -1);
    return s + "…";
  }

  // ---------------------------------------------------------------- opening
  function mono(text) { return el("span", "mono", text); }
  function renderOpening() {
    const m = D.meta;
    document.title = (m.repo ? m.repo + ": " : "") + "impact of this change";
    const c = $("context");
    add(c, m.repo ? el("span", "", m.repo) : null);
    if (m.since) {
      const rev = el("span", "mono", /^[0-9a-f]{40}$/.test(m.since) ? m.since.slice(0, 10) : m.since);
      rev.title = m.since;
      add(c, m.repo ? " · " : "", "changes since ", rev);
    }
    add(c, (m.repo || m.since ? " · " : "") + num(T.changes) + (T.changes === 1 ? " changed path" : " changed paths"));
    const h = $("claim");
    clear(h);
    if (chosen === 0) {
      add(h, "No " + noun(1) + " ", el("span", "quiet", "is selected by this change"));
    } else {
      add(h, num(chosen) + " of " + num(T.entries) + " " + noun(T.entries) + " ", el("span", "quiet", chosen === 1 ? "is selected by this change" : "are selected by this change"));
    }
    const l = $("lede");
    const bySource = new Map();
    for (const e of E) if (e[V] > 0 && e[S] >= 0) bySource.set(e[S], (bySource.get(e[S]) || 0) + 1);
    let top = -1, topN = 0;
    for (const [s, n] of bySource) if (D.sources[s].kind !== "site" && n > topN) { top = s; topN = n; }
    if (top >= 0 && topN >= Math.max(1, chosen * 0.15)) {
      const src = D.sources[top];
      add(l, num(topN) + (topN === 1 ? " traces back to " : " trace back to "), isPathSource(src) ? mono(src.label) : src.label);
      const byHub = new Map();
      for (const e of E) if (e[V] > 0 && e[S] === top && e[H] >= 0) byHub.set(e[H], (byHub.get(e[H]) || 0) + 1);
      let hub = -1, hubN = 0;
      for (const [x, n] of byHub) if (n > hubN) { hub = x; hubN = n; }
      if (hub >= 0 && hubN >= topN * 0.5 && hubN > 1) add(l, ", " + (hubN === topN ? "all" : num(hubN)) + " of them through ", mono(D.nodes[D.hubs[hub].node]));
      add(l, ". ");
    }
    const floor = D.causes.find((x) => x.id === "floor");
    if (floor && floor.first > 0) {
      add(l, num(floor.first) + (floor.first === 1 ? " runs on every change, because it reaches" : " run on every change, because they reach") + " code whose imports cannot be bounded. ");
    }
    if (T.unknown > 0) add(l, num(T.unknown) + " are reported as unknown. ");
    if (m.steering_only) add(l, "Only repository steering files changed, so uncertainty outside them does not count. ");
    if (T.unaffected > 0) add(l, num(T.unaffected) + (T.unaffected === 1 ? " does not run." : " do not run."));
    $("how-cap").textContent = chosen === 0 ? "The bar is all " + num(T.entries) + " " + noun(T.entries) + ". No changed file is among the files they load, read or are built from." :
      "The thin bar is all " + num(T.entries) + " " + noun(T.entries) + ". The bar below it spreads out the selected ones by the most direct reason each is selected; the hatched part is the uncertainty floor. Choose a reason to keep only its " + noun(2) + ".";
    $("foot-cmd").textContent = m.command;
    $("foot-note").textContent = m.tool + ". This file holds paths of the repository. It was written locally and loads nothing from the network.";
  }

  // ---------------------------------------------------------------- how much
  function toggleCause(i) {
    if (state.causes.has(i)) state.causes.delete(i); else state.causes.add(i);
    update();
  }
  function renderHow() {
    const bar = clear($("how-bar")), legend = clear($("how-legend"));
    bar.classList.toggle("has-pick", state.causes.size > 0);
    D.causes.forEach((c, i) => {
      if (!c.first) return;
      const on = state.causes.has(i);
      const seg = el("button", on ? "on" : "");
      seg.type = "button";
      seg.dataset.f = c.family;
      seg.style.flex = c.first + " 1 0";
      seg.tabIndex = -1;
      seg.setAttribute("aria-hidden", "true");
      seg.addEventListener("click", () => toggleCause(i));
      tipOn(seg, (t) => add(t, el("b", "", num(c.first) + " " + noun(c.first) + " · " + pct(c.first, T.entries)), el("span", "", c.label), el("span", "sub", c.about)));
      bar.append(seg);
      const li = el("li"), b = el("button");
      b.type = "button";
      b.setAttribute("aria-pressed", on);
      b.dataset.fk = "cause:" + i;
      const sw = el("span", "sw");
      sw.dataset.f = c.family;
      add(b, sw, el("span", "n", num(c.first)), el("span", "lab", c.label));
      b.addEventListener("click", () => toggleCause(i));
      tipOn(b, (t) => add(t, el("b", "", c.label), el("span", "sub", c.about), el("span", "sub", "Reaches " + num(c.inclusive) + ", alone for " + num(c.exclusive) + ".")));
      legend.append(add(li, b));
    });
    // All entries on one scale, and the selected ones spread out below it.
    const ends = clear($("how-ends")), total = clear($("how-total")), zoom = clear($("how-zoom"));
    add(ends, el("span", "", num(chosen) + " selected · " + pct(chosen, T.entries)));
    if (T.unaffected > 0) {
      const b = el("button", "", num(T.unaffected) + " not selected");
      b.type = "button";
      b.addEventListener("click", () => {
        state.verdict = "rest";
        update();
        // The answer is in the table: go there, with the focus.
        $("tests").scrollIntoView();
        const row = document.querySelector('#tests-table tr.row[tabindex="0"]');
        if (row) row.focus({ preventScroll: true });
      });
      ends.append(b);
    }
    const share = T.entries ? (100 * chosen) / T.entries : 0;
    if (chosen > 0) { const a = el("span", "sel"); a.style.flex = chosen + " 1 0"; total.append(a); }
    if (T.unaffected > 0) { const r = el("span", "rest"); r.style.flex = T.unaffected + " 1 0"; total.append(r); }
    zoom.append(svg("polygon", { points: "0,0 " + share + ",0 100,10 0,10" }));
    // Nothing selected: there is nothing to spread out.
    for (const id of ["how-zoom", "how-bar", "how-legend"]) $(id).style.display = chosen === 0 ? "none" : "";
  }

  // ---------------------------------------------------------------- the flow
  const familyOf = (list) => {
    const n = {};
    let best = "direct", bestN = -1;
    for (const i of list) {
      const f = D.causes[E[i][C]].family;
      n[f] = (n[f] || 0) + 1;
      if (n[f] > bestN) { bestN = n[f]; best = f; }
    }
    return best;
  };
  function column(list, key, limit, last) {
    const by = new Map();
    for (const i of list) {
      const k = key(E[i]);
      let a = by.get(k);
      if (!a) by.set(k, (a = []));
      a.push(i);
    }
    let nodes = [...by.entries()].map(([id, items]) => ({ id, items, n: items.length }));
    nodes.sort((a, b) => (a.id === last) - (b.id === last) || b.n - a.n || a.id - b.id);
    if (nodes.length > limit) {
      const keep = nodes.filter((x) => x.id !== last).slice(0, limit - 1 - (by.has(last) ? 1 : 0));
      const kept = new Set(keep.map((x) => x.id));
      const rest = nodes.filter((x) => x.id !== last && !kept.has(x.id));
      const other = { id: OTHER, items: [].concat(...rest.map((x) => x.items)), n: 0, more: rest.length };
      other.n = other.items.length;
      nodes = keep.concat(other, nodes.filter((x) => x.id === last));
    }
    return nodes;
  }
  function labelOf(col, node) {
    if (node.id === OTHER) return { text: num(node.more) + " more", path: false, quiet: true };
    if (col === 0) {
      const s = D.sources[node.id];
      return s ? { text: s.label, path: isPathSource(s) } : { text: "Unknown", path: false, quiet: true };
    }
    if (col === 1) return node.id === NOHUB ? { text: drilled() ? "Straight to the " + noun(2) : "No shared hub", path: false, quiet: true } : { text: D.nodes[D.hubs[node.id].node], path: true };
    return { text: D.groups[node.id].path, path: true };
  }
  function nodeTip(col, node) {
    return (t) => {
      add(t, el("b", "", num(node.n) + " " + noun(node.n)));
      const lab = labelOf(col, node);
      add(t, el("span", lab.path ? "mono" : "", lab.text));
      if (node.id === OTHER) return add(t, el("span", "sub", "Listed in the table below."));
      if (col === 0) {
        const s = D.sources[node.id];
        if (!s) return;
        const what = { site: "Open uncertainty: its " + noun(2) + " run on every change.", name: "A string in the code names a changed path.", package: "A dependency resolves differently.", layer: "", change: "", other: "" }[s.kind];
        if (what) add(t, el("span", "sub", what));
        add(t, el("span", "sub", "Only this: " + num(s.today) + (s.inclusive === null ? "" : " · reaches " + num(s.inclusive))));
      } else if (col === 1 && node.id !== NOHUB) {
        const h = D.hubs[node.id];
        add(t, el("span", "sub", "Every path to " + num(h.own) + " crosses it" + (h.inclusive === null ? "" : " · reaches " + num(h.inclusive))));
      } else if (col === 1) {
        add(t, el("span", "sub", drilled() ? "No further hub that every path crosses." : "Their chains share no hub that holds two or more."));
      } else {
        const g = D.groups[node.id];
        add(t, el("span", "sub", num(g.selected) + " of " + num(g.total) + " selected" + (g.name ? " · " + g.name : "")));
      }
    };
  }
  const keyOfCol = ["source", "hub", "group"];
  function pick(col, id) {
    if (id === OTHER || (col === 1 && id === NOHUB && drilled())) return;
    const k = keyOfCol[col];
    state[k] = state[k] === id ? -1 : id;
    update();
  }
  const isPicked = (col, id) => id !== OTHER && state[keyOfCol[col]] === id && !(col === 1 && state.hub === -1);
  function renderFlow() {
    const view = clear($("flow-view")), list = clear($("flow-list"));
    $("flow-cap").textContent =
      "Each selected " + noun(1) + " is drawn once, along the shortest chain of its most direct cause: from what changed, through the hub that holds the most " + noun(2) + " on that chain, to its package. Width is the number of " + noun(2) + ". Choose a block to keep only what passes through it.";
    if (!rows.length) {
      const msg = el("p", "flow-empty", chosen === 0 ? "Nothing is selected, so there is nothing to draw." : "No selected " + noun(1) + " matches the filters.");
      view.append(msg);
      list.append(msg.cloneNode(true));
      return;
    }
    const cols = [column(rows, (e) => e[S], 12, null), column(rows, hubOf, 11, NOHUB), column(rows, (e) => e[G], 12, null)];
    renderFlowList(list, cols);
    const W = Math.max(720, Math.min(1152, view.clientWidth || 1152));
    if (!view.clientWidth) return;
    // Each lane is as wide as its longest label needs, within a share of the width.
    const need = (col) => Math.max(...cols[col].map((x) => {
      const lab = labelOf(col, x);
      return width(lab.text, lab.path ? "path" : "n") + width(num(x.n), "n") + (col === 1 ? 48 : 30);
    }));
    const mark = 4;
    const laneL = Math.round(Math.min(W * 0.33, Math.max(120, need(0))));
    const laneM = Math.round(Math.min(W * 0.27, Math.max(170, need(1))));
    const laneR = Math.round(Math.min(W * 0.25, Math.max(110, need(2))));
    const gapW = (W - laneL - laneM - laneR) / 2;
    const xL = laneL, xM = laneL + gapW, xM2 = xM + laneM, xR = W - laneR;
    const top = 28, slot = 24, gap = 6;
    let unit = Math.min(14, 400 / rows.length);
    const heightOf = (c) => c.reduce((a, x) => a + Math.max(slot, x.n * unit), 0) + gap * (c.length - 1);
    for (let k = 0; k < 30 && Math.max(...cols.map(heightOf)) > 620 && unit * rows.length > 90; k++) unit *= 0.88;
    const Ht = Math.max(...cols.map(heightOf));
    for (const c of cols) {
      let y = top;
      for (const x of c) {
        x.h = Math.max(2, x.n * unit);
        x.slot = Math.max(slot, x.n * unit);
        x.y = y + (x.slot - x.h) / 2;
        x.cy = y + x.slot / 2;
        x.y0 = y;
        x.out = 0;
        x.inn = 0;
        y += x.slot + gap;
      }
    }
    const root = svg("svg", { viewBox: "0 0 " + W + " " + (Ht + top + 4), role: "group", "aria-label": "Flow from the changes through hubs to packages" });
    root.style.height = Ht + top + 4 + "px";
    const heads = [["What changed", xL - 10, "end"], [drilled() ? "Then through" : "Passes through", xM + 12, "start"], ["Runs in", xR + 12, "start"]];
    for (const [t, x, a] of heads) root.append(svg("text", { x, y: 12, "text-anchor": a, class: "quiet" }, t));
    // Ribbons.
    const idx = cols.map((c) => new Map(c.map((x, i) => [x.id, i])));
    const where = (col, e) => {
      const k = col === 0 ? e[S] : col === 1 ? hubOf(e) : e[G];
      return idx[col].has(k) ? idx[col].get(k) : idx[col].get(OTHER);
    };
    const ribs = [];
    for (const [a, b, x0, x1] of [[0, 1, xL + mark, xM], [1, 2, xM2, xR]]) {
      const links = new Map();
      for (const i of rows) {
        const s = where(a, E[i]), t = where(b, E[i]);
        const key = s * 4096 + t;
        let l = links.get(key);
        if (!l) links.set(key, (l = { s, t, items: [] }));
        l.items.push(i);
      }
      const all = [...links.values()];
      all.sort((p, q) => p.s - q.s || p.t - q.t);
      for (const l of all) { l.w = Math.max(1, l.items.length * unit); l.y0 = cols[a][l.s].y + cols[a][l.s].out; cols[a][l.s].out += l.items.length * unit; }
      all.sort((p, q) => p.t - q.t || p.s - q.s);
      for (const l of all) { l.y1 = cols[b][l.t].y + cols[b][l.t].inn; cols[b][l.t].inn += l.items.length * unit; }
      for (const l of all) {
        const mx = (x0 + x1) / 2;
        const d = "M" + x0 + "," + l.y0 + "C" + mx + "," + l.y0 + " " + mx + "," + l.y1 + " " + x1 + "," + l.y1 + "L" + x1 + "," + (l.y1 + l.w) + "C" + mx + "," + (l.y1 + l.w) + " " + mx + "," + (l.y0 + l.w) + " " + x0 + "," + (l.y0 + l.w) + "Z";
        const fam = familyOf(l.items);
        const g = svg("g", { class: "link" });
        const p = svg("path", { d, class: "rib" });
        p.dataset.f = fam;
        g.append(p);
        if (fam === "floor") {
          const tex = svg("path", { d, class: "rib-tex" });
          tex.style.fill = "url(#hatch)";
          g.append(tex);
        }
        const from = labelOf(a, cols[a][l.s]), to = labelOf(b, cols[b][l.t]);
        tipOn(g, (t) => add(t, el("b", "", num(l.items.length) + " " + noun(l.items.length)), el("span", from.path ? "mono" : "", from.text), el("span", to.path ? "mono" : "", "→ " + to.text), el("span", "sub", D.causes.find((c) => c.family === fam && l.items.some((i) => D.causes[E[i][C]] === c)).label + (new Set(l.items.map((i) => E[i][C])).size > 1 ? " and other causes" : ""))), false);
        g.addEventListener("pointerenter", () => light([[a, l.s], [b, l.t]], [p]));
        g.addEventListener("pointerleave", unlight);
        g.addEventListener("click", () => {
          const ia = cols[a][l.s].id, ib = cols[b][l.t].id;
          if (ia !== OTHER) state[keyOfCol[a]] = ia;
          if (ib !== OTHER) state[keyOfCol[b]] = ib;
          update();
        });
        ribs.push({ a, s: l.s, b, t: l.t, p });
        root.append(g);
      }
    }
    // Blocks.
    const groups = [[], [], []];
    cols.forEach((c, col) => {
      c.forEach((x, i) => {
        const lab = labelOf(col, x);
        const picked = isPicked(col, x.id);
        const g = svg("g", { class: "node" + (picked ? " on" : ""), tabindex: x.id === OTHER ? -1 : 0, role: "button", "aria-pressed": picked, "aria-label": lab.text + ", " + num(x.n) + " " + noun(x.n), "data-fk": "node:" + col + ":" + x.id });
        const x0 = col === 0 ? 0 : col === 1 ? xM : xR, wl = col === 0 ? laneL : col === 1 ? laneM : laneR;
        g.append(svg("rect", { class: "hit", x: x0, y: x.y0, width: wl, height: x.slot, rx: 3 }));
        const count = num(x.n), cw = width(count, "n");
        const cls = lab.path ? "path" : lab.quiet ? "quiet" : "";
        const ty = x.cy + 4.5;
        let tx, anchor, room;
        if (col === 0) {
          g.append(svg("rect", { class: "mark", x: xL, y: x.y, width: mark, height: x.h, rx: 1 }));
          g.append(svg("text", { class: "n", x: xL - 10, y: ty, "text-anchor": "end" }, count));
          tx = xL - 10 - cw - 10; anchor = "end"; room = tx - 2;
        } else if (col === 1) {
          g.append(svg("rect", { class: "band", x: xM, y: Math.min(x.y, x.cy - 11), width: laneM, height: Math.max(x.h, 22), rx: 2 }));
          g.append(svg("rect", { class: "mark", x: xM, y: x.y, width: mark, height: x.h, rx: 1 }));
          g.append(svg("rect", { class: "mark", x: xM2 - mark, y: x.y, width: mark, height: x.h, rx: 1 }));
          g.append(svg("text", { class: "n", x: xM2 - 12, y: ty, "text-anchor": "end" }, count));
          tx = xM + 12; anchor = "start"; room = laneM - 24 - cw - 10;
        } else {
          g.append(svg("rect", { class: "mark", x: xR, y: x.y, width: mark, height: x.h, rx: 1 }));
          g.append(svg("text", { class: "n", x: xR + 12, y: ty }, count));
          tx = xR + 12 + cw + 10; anchor = "start"; room = W - tx - 2;
        }
        const text = svg("text", { class: cls, x: tx, y: ty, "text-anchor": anchor });
        if (lab.path) {
          const [dir, name] = fitPath(lab.text, room, "path");
          if (dir) text.append(svg("tspan", { class: "dir" }, dir));
          text.append(svg("tspan", {}, name));
        } else {
          text.textContent = fitText(lab.text, room, cls || "n");
        }
        g.append(text);
        tipOn(g, nodeTip(col, x));
        const on = () => light([[col, i]], ribs.filter((r) => (r.a === col && r.s === i) || (r.b === col && r.t === i)).map((r) => r.p), ribs);
        g.addEventListener("pointerenter", on);
        g.addEventListener("focus", on);
        g.addEventListener("pointerleave", unlight);
        g.addEventListener("blur", unlight);
        g.addEventListener("click", () => pick(col, x.id));
        g.addEventListener("keydown", (ev) => {
          if (ev.key === "Enter" || ev.key === " ") { ev.preventDefault(); pick(col, x.id); }
        });
        groups[col].push(g);
        root.append(g);
      });
    });
    function light(nodes, paths, all) {
      view.classList.add("hl");
      for (const p of paths) p.classList.add("lit");
      const lit = new Set(nodes.map(([c, i]) => c + ":" + i));
      if (all) for (const r of all) if (paths.includes(r.p)) { lit.add(r.a + ":" + r.s); lit.add(r.b + ":" + r.t); }
      groups.forEach((c, col) => c.forEach((g, i) => g.classList.toggle("lit", lit.has(col + ":" + i))));
    }
    function unlight() {
      view.classList.remove("hl");
      for (const r of ribs) r.p.classList.remove("lit");
    }
    view.append(root);
  }
  // The same three columns as lists, for a narrow screen.
  function renderFlowList(host, cols) {
    const names = ["What changed", drilled() ? "Then through" : "Passes through", "Runs in"];
    const wrap = el("div", "bars");
    cols.forEach((c, col) => {
      wrap.append(el("h3", "", names[col]));
      const max = Math.max(...c.map((x) => x.n));
      for (const x of c) {
        const lab = labelOf(col, x);
        const r = el("button", "r");
        r.type = "button";
        if (x.id === OTHER) r.disabled = true;
        r.setAttribute("aria-pressed", isPicked(col, x.id));
        r.dataset.fk = "row:" + col + ":" + x.id;
        const l = el("span", "lab" + (lab.path ? " mono" : "") + (lab.quiet ? " dim" : ""));
        if (lab.path) l.append(pathNode(lab.text)); else l.textContent = lab.text;
        if (lab.path) { l.style.direction = "rtl"; l.style.textAlign = "left"; }
        const track = el("span", "track");
        track.style.gridColumn = "1 / -1";
        track.style.marginTop = "6px";
        const fill = el("span", "fill");
        fill.style.width = Math.max(1, (100 * x.n) / max) + "%";
        track.append(fill);
        add(r, l, el("span", "num", num(x.n)), track);
        r.addEventListener("click", () => pick(col, x.id));
        wrap.append(r);
      }
    });
    host.append(wrap);
  }

  // ---------------------------------------------------------------- tables
  // A table whose body holds only the rows in view.
  const tables = [];
  function Table(host, cols, opts) {
    const ROW = 36;
    const wrap = el("div", "tw"), table = el("table"), head = el("thead"), body = el("tbody");
    let data = [], sort = opts.sort || null, first = -1, last = -1;
    if (opts.label) table.setAttribute("aria-label", opts.label);
    if (!opts.onRow) table.classList.add("static");
    const hr = el("tr");
    cols.forEach((c, i) => {
      // Widths sit on the header cells: a column hidden on a narrow screen then takes no room.
      const th = el("th", (c.num ? "num " : "") + (c.cls || ""));
      th.scope = "col";
      if (c.w && c.w !== "auto") th.style.width = c.w;
      if (c.sort) {
        const b = el("button");
        b.type = "button";
        add(b, c.label, el("span", "arr", ""));
        b.addEventListener("click", () => {
          sort = sort && sort.i === i ? { i, dir: -sort.dir } : { i, dir: c.num ? -1 : 1 };
          apply();
        });
        if (c.title) b.title = c.title;
        th.append(b);
      } else th.textContent = c.label;
      hr.append(th);
    });
    head.append(hr);
    add(table, head, body);
    // The columns that are shown (some are hidden on a narrow screen).
    const span = () => [...hr.children].filter((th) => getComputedStyle(th).display !== "none").length || cols.length;
    wrap.append(table);
    const foot = el("div", "tfoot");
    add(host, wrap, foot);
    function apply() {
      if (sort) {
        const c = cols[sort.i], dir = sort.dir;
        const keyed = data.map((r) => [c.sort(r), r]);
        keyed.sort((a, b) => (a[0] < b[0] ? -dir : a[0] > b[0] ? dir : 0));
        data = keyed.map((x) => x[1]);
      }
      [...hr.children].forEach((th, i) => {
        const on = sort && sort.i === i;
        if (on) th.setAttribute("aria-sort", sort.dir > 0 ? "ascending" : "descending"); else th.removeAttribute("aria-sort");
        const arr = th.querySelector(".arr");
        if (arr) arr.textContent = on ? (sort.dir > 0 ? "↑" : "↓") : "";
      });
      wrap.classList.toggle("short", data.length <= 12);
      first = last = -1;
      wrap.scrollTop = 0;
      draw();
      clear(foot);
      if (opts.foot) opts.foot(foot, data);
    }
    function draw(force) {
      const virtual = data.length > 12;
      const a = virtual ? Math.max(0, Math.floor(wrap.scrollTop / ROW) - 6) : 0;
      const b = virtual ? Math.min(data.length, a + Math.ceil(wrap.clientHeight / ROW) + 14) : data.length;
      if (!force && a === first && b === last) return;
      first = a; last = b;
      const focus = document.activeElement && body.contains(document.activeElement) ? +document.activeElement.dataset.i : -1;
      clear(body);
      if (!data.length) {
        const tr = el("tr"), td = el("td", "dim", opts.empty || "Nothing to show.");
        td.colSpan = span();
        body.append(add(tr, td));
        return;
      }
      const shown = span();
      const spacer = (h) => { const tr = el("tr", "sp"), td = el("td"); td.colSpan = shown; td.style.height = h + "px"; tr.setAttribute("aria-hidden", "true"); return add(tr, td); };
      if (a > 0) body.append(spacer(a * ROW));
      for (let i = a; i < b; i++) {
        const r = data[i], tr = el("tr", "row");
        // One stop per table: the open row, or the first one. The arrow keys move.
        tr.tabIndex = -1;
        tr.dataset.i = i;
        if (typeof r === "number") tr.dataset.fk = (opts.label || "t") + ":" + r;
        if (opts.selected) tr.setAttribute("aria-selected", opts.selected(r));
        for (const c of cols) {
          const td = el("td", (c.num ? "num " : "") + (c.cls || "") + (c.path ? " p" : ""));
          c.cell(r, td);
          tr.append(td);
        }
        body.append(tr);
        if (i === focus) tr.focus({ preventScroll: true });
      }
      const stop = (focus >= a && focus < b && body.querySelector('tr.row[data-i="' + focus + '"]')) || body.querySelector('tr.row[aria-selected="true"]') || body.querySelector("tr.row");
      if (stop) stop.tabIndex = 0;
      if (b < data.length) body.append(spacer((data.length - b) * ROW));
    }
    wrap.addEventListener("scroll", () => draw(), { passive: true });
    body.addEventListener("click", (ev) => {
      const tr = ev.target.closest("tr.row");
      if (tr && opts.onRow) opts.onRow(data[+tr.dataset.i]);
    });
    body.addEventListener("keydown", (ev) => {
      const tr = ev.target.closest("tr.row");
      if (!tr) return;
      const i = +tr.dataset.i;
      if (ev.key === "Enter" || ev.key === " ") { ev.preventDefault(); if (opts.onRow) opts.onRow(data[i]); return; }
      const to = ev.key === "ArrowDown" ? i + 1 : ev.key === "ArrowUp" ? i - 1 : ev.key === "Home" ? 0 : ev.key === "End" ? data.length - 1 : -1;
      if (to < 0 || to >= data.length) return;
      ev.preventDefault();
      const y = to * ROW;
      if (y < wrap.scrollTop) wrap.scrollTop = y; else if (y + ROW + 40 > wrap.scrollTop + wrap.clientHeight) wrap.scrollTop = y + ROW + 40 - wrap.clientHeight;
      draw(true);
      const next = body.querySelector('tr.row[data-i="' + to + '"]');
      if (next) {
        for (const r of body.querySelectorAll('tr.row[tabindex="0"]')) r.tabIndex = -1;
        next.tabIndex = 0;
        next.focus({ preventScroll: true });
      }
    });
    const api = {
      set(next) { data = next.slice(); apply(); },
      refresh() { draw(true); },
      first() { return data[0]; },
    };
    tables.push(api);
    return api;
  }
  const pathCell = (p) => (r, td) => { const v = p(r); td.append(pathNode(v)); td.title = v; };
  const numCell = (f) => (r, td) => { const v = f(r); td.textContent = v === null || v === undefined ? "–" : num(v); };
  function meterCell(f, max) {
    return (r, td) => {
      const v = f(r);
      const m = el("span", "meter"), i = el("i");
      i.style.width = (max() ? Math.max(2, (100 * v) / max()) : 0) + "%";
      add(td, num(v), add(m, i));
    };
  }

  // Sources.
  let srcCount = new Map(), srcMax = 1, hubCount = new Map(), hubMax = 1;
  const sourcesTable = Table($("sources-table"), [
    { label: "Source", w: "auto", path: true, sort: (s) => D.sources[s].label, cell: (s, td) => { const x = D.sources[s]; if (isPathSource(x)) td.append(pathNode(x.label)); else { td.classList.remove("p"); td.textContent = x.label; } td.title = x.label; } },
    { label: "Kind", w: "170px", cls: "hide-s", sort: (s) => sourceKind(D.sources[s]), cell: (s, td) => { td.textContent = sourceKind(D.sources[s]); td.classList.add("dim"); } },
    { label: "Selects", w: "150px", num: true, sort: (s) => srcCount.get(s) || 0, title: "Entries drawn from it: each selected entry counts under one source", cell: meterCell((s) => srcCount.get(s) || 0, () => srcMax) },
    { label: "Only this", w: "96px", num: true, sort: (s) => D.sources[s].today, title: "Entries that are not selected without it", cell: numCell((s) => D.sources[s].today) },
    { label: "Reaches", w: "96px", num: true, cls: "hide-s", sort: (s) => (D.sources[s].inclusive === null ? -1 : D.sources[s].inclusive), title: "Every entry it reaches, also those counted under another source", cell: numCell((s) => D.sources[s].inclusive) },
  ], {
    label: "Sources of the selection",
    sort: { i: 2, dir: -1 },
    selected: (s) => state.source === s,
    onRow: (s) => { state.source = state.source === s ? -1 : s; update(); },
    empty: "No source matches the filters.",
  });
  const hubsTable = Table($("hubs-table"), [
    { label: "Hub", w: "auto", path: true, sort: (h) => D.nodes[D.hubs[h].node], cell: pathCell((h) => D.nodes[D.hubs[h].node]) },
    { label: "Kind", w: "190px", cls: "hide-s", sort: (h) => D.hubs[h].kind + D.hubs[h].mech, cell: (h, td) => { const x = D.hubs[h]; td.textContent = (x.kind === "members" ? "files of a package" : x.kind === "part" ? "part of a task" : x.kind) + (x.mech === "floor" ? ", on the floor" : ""); td.classList.add("dim"); } },
    { label: "Passes", w: "150px", num: true, sort: (h) => hubCount.get(h) || 0, title: "Selected entries whose chain crosses it", cell: meterCell((h) => hubCount.get(h) || 0, () => hubMax) },
    { label: "Only through", w: "110px", num: true, sort: (h) => D.hubs[h].own, title: "Entries that every path of their kind crosses it for", cell: numCell((h) => D.hubs[h].own) },
    { label: "Reaches", w: "96px", num: true, cls: "hide-s", sort: (h) => (D.hubs[h].inclusive === null ? -1 : D.hubs[h].inclusive), title: "Every entry with a path through it", cell: numCell((h) => D.hubs[h].inclusive) },
  ], {
    label: "Hubs between the changes and the entries",
    sort: { i: 2, dir: -1 },
    selected: (h) => state.hub === h,
    onRow: (h) => { state.hub = state.hub === h ? -1 : h; update(); },
    empty: "No hub holds two or more of these entries.",
  });
  function renderSourcesAndHubs() {
    srcCount = new Map();
    hubCount = new Map();
    for (const i of base) {
      const e = E[i];
      if (e[S] >= 0) srcCount.set(e[S], (srcCount.get(e[S]) || 0) + 1);
      if (e[H] >= 0) hubCount.set(e[H], (hubCount.get(e[H]) || 0) + 1);
      for (const h of e[DOM]) if (h !== e[H]) hubCount.set(h, (hubCount.get(h) || 0) + 1);
    }
    srcMax = Math.max(1, ...srcCount.values());
    hubMax = Math.max(1, ...hubCount.values());
    sourcesTable.set([...srcCount.keys()]);
    hubsTable.set([...hubCount.keys()]);
    $("sources-cap").textContent =
      "Selects counts each selected " + noun(1) + " once, under the source its chain starts at. Only this: " + noun(2) + " that are not selected without it. Reaches: every " + noun(1) + " it reaches, also those counted under another source.";
    $("hubs-cap").textContent =
      "A hub is a file or task between a change and several " + noun(2) + ": a barrel, a setup file, a shared helper. Only through: " + noun(2) + " for which every path crosses it, so they leave the selection if that edge goes. A large number next to a small change is the place to look.";
  }
  function renderIdle() {
    const host = clear($("idle"));
    const used = new Set();
    for (const e of E) if (e[V] > 0 && e[S] >= 0) used.add(e[S]);
    const idle = D.changes.filter((c) => c.source === null || !used.has(c.source));
    host.hidden = idle.length === 0;
    if (!idle.length) return;
    host.append(el("summary", "", num(idle.length) + " of " + num(D.changes.length) + " changed paths are the first cause of nothing"));
    const t = Table(host, [
      { label: "Changed path", w: "40%", path: true, sort: (c) => c.path, cell: pathCell((c) => c.path) },
      { label: "What became of it", w: "auto", sort: (c) => c.status, cell: (c, td) => {
        const reach = c.source !== null ? D.sources[c.source].inclusive : null;
        td.textContent = c.note || (c.source !== null ? "Reaches " + (reach === null ? "" : num(reach) + " ") + (reach === 1 ? noun(1) + " that a more direct cause already selects" : noun(2) + " that a more direct cause already selects") : "No " + noun(1) + " depends on it");
        td.title = td.textContent;
        td.classList.add("dim");
      } },
    ], { label: "Changed paths that select nothing by themselves" });
    t.set(idle);
    host.addEventListener("toggle", () => t.refresh());
  }

  // Entries.
  const causeCell = (i, td) => {
    const e = E[i];
    if (e[V] === 0) { td.textContent = "Not selected"; td.classList.add("dim"); return; }
    const c = D.causes[e[C]];
    const w = el("span", "cause"), sw = el("span", "sw");
    sw.dataset.f = c.family;
    add(td, add(w, sw, el("span", "", c.label + (e[V] === 2 ? " (unknown)" : ""))));
  };
  const testsTable = Table($("tests-table"), [
    { label: allTests ? "Test" : allTasks ? "Task" : "Entry", w: "auto", path: true, sort: (i) => D.nodes[E[i][P]], cell: pathCell((i) => D.nodes[E[i][P]]) },
    { label: "Why", w: "190px", sort: (i) => (E[i][V] === 0 ? 99 : E[i][C]), cell: causeCell },
    { label: "From", w: "24%", cls: "hide-s", path: true, sort: (i) => (E[i][S] >= 0 ? D.sources[E[i][S]].label : "~"), cell: (i, td) => {
      const e = E[i];
      if (e[S] < 0) { td.textContent = ""; return; }
      const s = D.sources[e[S]];
      if (isPathSource(s)) td.append(pathNode(s.label)); else { td.classList.remove("p"); td.textContent = s.label; }
      td.title = s.label;
    } },
  ], {
    label: "Entries",
    selected: (i) => state.open === i,
    onRow: (i) => { state.open = i; testsTable.refresh(); renderDetail(true); },
    empty: "No entry matches the filters.",
    foot: (f, data) => {
      const total = state.verdict === "selected" ? chosen : state.verdict === "rest" ? T.unaffected : E.length;
      add(f, el("span", "", num(data.length) + (data.length === total ? " " : " of " + num(total) + " ") + noun(total)), el("span", "hide-s", "Arrow keys move, Enter opens"));
    },
  });
  function renderVerdict() {
    const host = clear($("verdict"));
    for (const [id, label, n] of [["selected", "Selected", chosen], ["rest", "Not selected", T.unaffected], ["all", "All", E.length]]) {
      const b = el("button");
      b.type = "button";
      add(b, el("span", "", label), el("span", "n", num(n)));
      b.setAttribute("aria-pressed", state.verdict === id);
      b.dataset.fk = "verdict:" + id;
      b.addEventListener("click", () => { state.verdict = id; update(); });
      host.append(b);
    }
  }
  function renderTests() {
    renderVerdict();
    let data = [];
    if (state.verdict !== "rest") data = rows.slice();
    if (state.verdict !== "selected") {
      const q = state.q.trim().toLowerCase();
      const narrowed = state.causes.size > 0 || state.source >= 0 || state.hub !== -1;
      if (!narrowed || state.verdict === "rest") {
        for (let i = 0; i < E.length; i++) {
          const e = E[i];
          if (e[V] !== 0) continue;
          if (q && !low(i).includes(q)) continue;
          if (state.group >= 0 && e[G] !== state.group) continue;
          data.push(i);
        }
      }
      if (state.verdict === "all") data.sort((a, b) => (D.nodes[E[a][P]] < D.nodes[E[b][P]] ? -1 : 1));
    }
    testsTable.set(data);
    // One entry is always open: the page shows a chain at rest.
    if (state.open < 0 || !data.includes(state.open)) state.open = data.length ? testsTable.first() : -1;
    testsTable.refresh();
    renderDetail(false);
  }
  const hubByNode = [new Map(), new Map()];
  D.hubs.forEach((h, i) => hubByNode[h.mech === "floor" ? 1 : 0].set(h.node, i));
  function explainCommand(path) {
    const drop = /^(--html(=.*)?|--open|--stats|--exit-code)$/;
    const parts = D.meta.command.split(" ").filter((a) => !drop.test(a));
    const f = parts.indexOf("--format");
    if (f >= 0) parts.splice(f, 2);
    return parts.filter((a) => !a.startsWith("--format=")).join(" ") + " --explain " + path;
  }
  function renderDetail(scroll) {
    const host = clear($("detail"));
    if (state.open < 0) {
      host.append(el("p", "hint", "Choose a row to see why it is selected: the chain from the change to it, the hubs on the way, and what else reaches it."));
      return;
    }
    const e = E[state.open], path = D.nodes[e[P]];
    host.append(el("h3", "", path));
    if (e[V] === 0) {
      const why = el("div", "why");
      add(why, el("div", "cause", "Not selected"));
      const ul = el("ul", "plain");
      if (e[U].length) for (const t of e[U]) ul.append(el("li", "text", D.texts[t]));
      else ul.append(el("li", "text dim", "No changed file is in what it loads" + (D.meta.floor_applies ? ", and it reaches no open uncertainty." : ".")));
      add(host, why, ul);
    } else {
      const c = D.causes[e[C]];
      const why = el("div", "why"), cause = el("div", "cause"), sw = el("span", "sw");
      sw.dataset.f = c.family;
      add(cause, sw, el("span", "", c.label));
      add(why, cause, el("p", "dim", c.about));
      host.append(why);
      const mech = c.id === "floor" ? 1 : 0;
      const src = e[S] >= 0 ? D.sources[e[S]] : null;
      if (e[CH].length) {
        const box = el("div");
        box.append(el("h4", "", e[EW].length ? "Shortest chain of this kind" : "Chain"));
        const ol = el("ol", "chain");
        e[CH].forEach((nd, k) => {
          const p = D.nodes[nd], li = el("li", k === 0 ? "start" : k === e[CH].length - 1 ? "end" : "");
          const hub = hubByNode[mech].get(nd);
          const lastOne = k === e[CH].length - 1;
          if (hub !== undefined && !lastOne && k > 0) {
            const b = el("button", "", p);
            b.type = "button";
            b.addEventListener("click", () => { state.hub = hub; update(); $("flow").scrollIntoView(); });
            li.append(b);
            const h = D.hubs[hub];
            li.append(el("small", "", (e[DOM].includes(hub) ? "Hub. Every path to this " + noun(1) + " crosses it. " : "Hub. ") + num(h.own) + " " + noun(h.own) + (h.own === 1 ? " depends" : " depend") + " on it alone."));
          } else {
            li.append(p);
            if (k === 0 && src && src.label === p) li.append(el("small", "", src.kind === "site" ? "Open uncertainty: what it loads cannot be bounded" : "Changed" + (e[X] ? ". Nothing else that changed reaches this " + noun(1) + "." : "")));
            else if (lastOne && e[CH].length > 1) li.append(el("small", "", "This " + noun(1)));
            else if (k === 0 && src) li.append(el("small", "", (src.kind === "package" ? "Links " : src.kind === "site" ? "Open uncertainty in " : "Affected by ") + src.label));
          }
          ol.append(li);
        });
        add(host, add(box, ol));
      }
      const y = e[Y] >= 0 ? D.texts[e[Y]] : "";
      if (e[EW].length || (y && y !== "modified" && y !== "deleted")) {
        const box = el("div");
        box.append(el("h4", "", "What --explain prints"));
        const ul = el("ul", "plain");
        const w = (e[EW].length ? e[EW] : e[CH]).map((nd) => D.nodes[nd]).join(" → ");
        const lines = y.split("\n");
        ul.append(el("li", "", (lines[0] ? lines[0] + " " : "") + w));
        for (const extra of lines.slice(1)) ul.append(el("li", "dim", extra));
        add(host, add(box, ul));
      }
      const others = e[O].filter((s) => s !== e[S]);
      if (others.length) {
        const box = el("div");
        const more = e[ON] - (e[O].includes(e[S]) ? 1 : 0);
        box.append(el("h4", "", "Also reached from"));
        const ul = el("ul", "plain");
        for (const s of others.slice(0, 8)) ul.append(el("li", isPathSource(D.sources[s]) ? "" : "text", D.sources[s].label));
        if (more > Math.min(8, others.length)) ul.append(el("li", "text dim", "and " + num(more - Math.min(8, others.length)) + " more"));
        add(host, add(box, ul));
      }
      const also = D.causes.filter((x, i) => i !== e[C] && e[CM] & (1 << i));
      if (also.length) {
        const box = el("div");
        box.append(el("h4", "", "Less direct causes that also apply"));
        add(host, add(box, el("p", "", also.map((x) => x.label).join(", "))));
      }
    }
    if (e[V] !== 0 || e[U].length) {
      const box = el("div");
      box.append(el("h4", "", "In the terminal"));
      add(host, add(box, el("p", "cmd", explainCommand(path))));
    }
    if (scroll && window.matchMedia("(max-width: 900px)").matches) host.scrollIntoView({ block: "nearest" });
  }

  // ---------------------------------------------------------------- the floor
  function renderFloor() {
    const f = D.floor, host = clear($("floor-body")), stack = el("div", "stack");
    const cap = $("floor-cap");
    cap.textContent = "Code that loads a module from a computed name can load anything, so no change can be ruled out for the " + noun(2) + " that reach it. They are the floor of every selection.";
    if (!f.floor) {
      stack.append(el("p", "fact", "No " + noun(1) + " reaches open uncertainty. Nothing is selected on every change."));
      host.append(stack);
      return;
    }
    const fact = el("p", "fact");
    add(fact, num(f.floor) + " of " + num(f.entries) + " " + noun(f.entries) + " (" + pct(f.floor, f.entries) + ") reach open uncertainty in " + num(f.open_sites_reached) + (f.open_sites_reached === 1 ? " file" : " files") + (D.meta.floor_applies ? "." : ". The policy of this run does not select them for it."));
    stack.append(fact);
    const sites = new Map(f.sites.map((s) => [s.path, s]));
    if (f.greedy && f.greedy.length) {
      const box = el("div", "section-head");
      box.append(el("h3", "", "Resolve these first"));
      box.append(el("p", "caption", "In this order each site frees the most " + noun(2) + ": freed leave the floor once the site is bounded, left stay on it because of another site."));
      const ol = el("ol", "steps");
      const head = el("li", "head");
      add(head, el("span", "", ""), el("span", "", "Site"), el("span", "num", "Freed"), el("span", "num left", "Left"));
      ol.append(head);
      f.greedy.forEach((g, i) => {
        const li = el("li");
        const site = el("span", "site", g.path);
        add(li, el("span", "i", String(i + 1)), site, el("span", "num v", num(g.freed)), el("span", "num left", num(g.remaining)));
        const s = sites.get(g.path);
        if (s && s.records.length) {
          const recs = el("span", "recs");
          for (const r of s.records.slice(0, 3)) recs.append(el("span", "", "line " + r.line + " · " + r.kind + " / " + r.effect + " / " + r.origin + (r.detail ? " · " + r.detail : "")));
          li.append(recs);
        }
        ol.append(li);
      });
      add(stack, box, ol);
    }
    if (f.sites.length) {
      const d = el("details", "more");
      d.append(el("summary", "", "All " + num(f.open_sites) + " files with open records"));
      const t = Table(d, [
        { label: "File", w: "auto", path: true, sort: (s) => s.path, cell: pathCell((s) => s.path) },
        { label: "Records", w: "90px", num: true, cls: "hide-s", sort: (s) => s.records.length, cell: numCell((s) => s.records.length) },
        { label: "Reached by", w: "110px", num: true, sort: (s) => s.reached, title: "Entries that reach it", cell: numCell((s) => s.reached) },
        { label: "Only this", w: "100px", num: true, sort: (s) => s.exclusive, title: "Entries that reach no other site", cell: numCell((s) => s.exclusive) },
      ], { label: "Files with open uncertainty", sort: { i: 2, dir: -1 } });
      t.set(f.sites);
      d.addEventListener("toggle", () => t.refresh());
      stack.append(d);
    }
    host.append(stack);
  }

  // ---------------------------------------------------------------- not selected, tasks
  function groupBars(host, list, label) {
    const wrap = el("div", "bars wide");
    const max = Math.max(1, ...list.map((g) => g.total));
    for (const g of list) {
      const r = el(g.pick ? "button" : "div", "r");
      if (g.pick) { r.type = "button"; r.addEventListener("click", g.pick); }
      const l = el("span", "lab mono");
      l.append(pathNode(g.label));
      l.title = g.label;
      const tr = el("span", "track2"), tot = el("i", "tot"), sel = el("i", "sel");
      tot.style.width = (100 * g.total) / max + "%";
      sel.style.width = (100 * g.selected) / max + "%";
      add(tr, tot, sel);
      add(r, l, tr, el("span", "num", label(g)));
      wrap.append(r);
    }
    host.append(wrap);
  }
  function renderRest() {
    const host = clear($("rest-body")), stack = el("div", "stack");
    const p = D.pruning;
    $("rest-cap").textContent = "A " + noun(1) + " is left out when no changed file is among the files it loads, reads or is built from" + (D.meta.floor_applies ? ", and it reaches no open uncertainty." : ".");
    const fact = el("p", "fact", num(T.unaffected) + " of " + num(T.entries) + " " + noun(T.entries) + " (" + pct(T.unaffected, T.entries) + ") do not run.");
    const noted = E.reduce((a, e) => a + (e[V] === 0 && e[U].length ? 1 : 0), 0);
    if (noted) {
      const parts = [];
      if (p.erased) parts.push(num(p.erased) + (p.erased === 1 ? " file whose JavaScript is unchanged" : " files whose JavaScript is unchanged"));
      if (p.barrels) parts.push(num(p.barrels) + (p.barrels === 1 ? " file behind a barrel" : " files behind a barrel"));
      if (p.type_only) parts.push("type-only imports");
      add(fact, " " + num(noted) + " of them load a changed file that cannot affect them: " + parts.join(", ") + ". Open one below to see which.");
    }
    stack.append(fact);
    const list = D.groups.map((g, i) => ({ label: g.path, total: g.total, selected: g.selected, pick: () => { state.group = state.group === i ? -1 : i; state.verdict = "all"; update(); $("tests").scrollIntoView(); } }))
      .sort((a, b) => b.total - b.selected - (a.total - a.selected) || b.total - a.total);
    const box = el("div", "section-head");
    box.append(el("h3", "", "By package"));
    box.append(el("p", "caption", "The dark part of each bar runs, the light part does not. One scale for all rows."));
    stack.append(box);
    const shown = list.slice(0, 14);
    groupBars(stack, shown, (g) => num(g.total - g.selected) + " of " + num(g.total) + " left out");
    if (list.length > shown.length) {
      // Drawn when opened: a large repository has a thousand of them.
      const d = el("details", "more");
      d.append(el("summary", "", num(list.length - shown.length) + " more packages"));
      d.addEventListener("toggle", () => {
        if (d.open && d.children.length === 1) groupBars(d, list.slice(14), (g) => num(g.total - g.selected) + " of " + num(g.total) + " left out");
      });
      stack.append(d);
    }
    host.append(stack);
  }
  function renderTasks() {
    const k = D.kinds.findIndex((x) => x.id === "task");
    if (k < 0) return false;
    $("tasks").hidden = false;
    const by = new Map();
    for (const e of E) {
      if (e[K] !== k) continue;
      const p = D.nodes[e[P]];
      const name = p.includes("#") ? p.slice(p.lastIndexOf("#") + 1) : p.split(":")[0];
      let g = by.get(name);
      if (!g) by.set(name, (g = { label: name, total: 0, selected: 0 }));
      g.total++;
      if (e[V] > 0) g.selected++;
    }
    const list = [...by.values()].sort((a, b) => b.selected - a.selected || b.total - a.total).slice(0, 24);
    const x = D.kinds[k];
    $("tasks-cap").textContent = num(x.selected) + " of " + num(x.total) + " build tasks are selected. By task name, on one scale: the dark part runs.";
    groupBars(clear($("tasks-body")), list, (g) => num(g.selected) + " of " + num(g.total));
    return true;
  }

  // ---------------------------------------------------------------- the bar
  function renderFilters() {
    const host = clear($("filters"));
    host.hidden = !filtered();
    if (host.hidden) return;
    host.append(el("span", "count", num(rows.length) + " of " + num(chosen) + " selected " + noun(chosen)));
    const chip = (label, value, off) => {
      const b = el("button", "chip");
      b.type = "button";
      b.setAttribute("aria-label", "Remove filter " + label + " " + value);
      add(b, el("span", "dim", label), el("b", "", value), el("span", "x", "×"));
      b.addEventListener("click", () => { off(); update(); });
      host.append(b);
    };
    if (state.q.trim()) chip("Path", state.q.trim(), () => { state.q = ""; $("q").value = ""; });
    for (const c of state.causes) chip("Why", D.causes[c].label, () => state.causes.delete(c));
    if (state.source >= 0) chip("From", D.sources[state.source].label, () => { state.source = -1; });
    if (state.hub !== -1) chip("Through", state.hub === NOHUB ? "no shared hub" : D.nodes[D.hubs[state.hub].node], () => { state.hub = -1; });
    if (state.group >= 0) chip("In", D.groups[state.group].path, () => { state.group = -1; });
    const c = el("button", "clear", "Clear");
    c.type = "button";
    c.addEventListener("click", () => { state.q = ""; $("q").value = ""; state.causes.clear(); state.source = -1; state.hub = -1; state.group = -1; update(); });
    host.append(c);
  }
  function renderNav(hasTasks) {
    const nav = $("nav");
    let items = [["flow", "Flow"], ["sources", "Sources"], ["hubs", "Hubs"], ["tests", allTests ? "Tests" : "Entries"], ["floor", "Every change"], ["rest", "Not selected"]];
    if (chosen === 0) {
      // Nothing is selected: no sources and no hubs to list.
      items = items.filter(([id]) => id !== "sources" && id !== "hubs");
      $("sources").hidden = $("hubs").hidden = true;
    }
    if (hasTasks) items.push(["tasks", "Tasks"]);
    const links = new Map();
    for (const [id, label] of items) {
      const a = el("a", "", label);
      a.href = "#" + id;
      nav.append(a);
      links.set(id, a);
    }
    if (!("IntersectionObserver" in window)) return;
    const seen = new Set();
    const io = new IntersectionObserver((list) => {
      for (const x of list) if (x.isIntersecting) seen.add(x.target.id); else seen.delete(x.target.id);
      let cur = null;
      for (const [id] of items) if (seen.has(id)) { cur = id; break; }
      for (const [id, a] of links) if (id === cur) a.setAttribute("aria-current", "true"); else a.removeAttribute("aria-current");
    }, { rootMargin: "-120px 0px -55% 0px" });
    for (const [id] of items) io.observe($(id));
  }

  // ---------------------------------------------------------------- wiring
  function update() {
    // The element that has the focus is drawn again: find it by what it stands for.
    const a = document.activeElement;
    const fk = a && a.dataset ? a.dataset.fk : null;
    compute();
    renderHow();
    renderFilters();
    renderFlow();
    renderSourcesAndHubs();
    renderTests();
    if (fk && (!document.activeElement || document.activeElement === document.body)) {
      for (const e of document.querySelectorAll("[data-fk]")) {
        if (e.dataset.fk === fk && e.getClientRects().length) { e.focus({ preventScroll: true }); break; }
      }
    }
    hideTip();
  }
  let timer = 0;
  $("q").addEventListener("input", (ev) => {
    state.q = ev.target.value;
    clearTimeout(timer);
    timer = setTimeout(update, 90);
  });
  $("q").addEventListener("keydown", (ev) => {
    if (ev.key === "Escape" && state.q) { state.q = ""; ev.target.value = ""; update(); }
  });
  document.addEventListener("keydown", (ev) => {
    if (ev.key === "/" && !ev.metaKey && !ev.ctrlKey && document.activeElement !== $("q")) { ev.preventDefault(); $("q").focus(); }
  });
  let lastW = 0;
  window.addEventListener("resize", () => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      if (window.innerWidth === lastW) return;
      lastW = window.innerWidth;
      renderFlow();
      for (const t of tables) t.refresh();
    }, 120);
  });

  renderOpening();
  $("tests-h").textContent = allTests ? "Every test" : allTasks ? "Every task" : "Every entry";
  const hasTasks = renderTasks();
  renderNav(hasTasks);
  renderIdle();
  renderFloor();
  renderRest();
  update();
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(() => { for (const k in fonts) delete fonts[k]; renderFlow(); });
})();
