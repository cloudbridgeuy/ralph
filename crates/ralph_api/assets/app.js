"use strict";

const SCREENS = ["/memory"];
const ROUTE = SCREENS[0];
const PAGE_SIZE = 20;
const SVG_NS = "http://www.w3.org/2000/svg";
const EXTENT_MIN = 1;
const SPINNER_DELAY_MS = 200;

function parseHash(hash) {
  const body = hash.replace(/^#/, "");
  const cut = body.indexOf("?");
  const route = cut < 0 ? body : body.slice(0, cut);
  const query = new URLSearchParams(cut < 0 ? "" : body.slice(cut + 1));
  const raw = query.get("after");
  const after = raw === null || raw === "" ? NaN : Number(raw);
  return {
    route,
    thread: query.get("thread") || "main",
    node: query.get("node") || "",
    after: Number.isInteger(after) && after >= 0 ? after : null,
  };
}

function buildHash(state) {
  const query = new URLSearchParams();
  if (state.thread && state.thread !== "main") query.set("thread", state.thread);
  if (state.node) query.set("node", state.node);
  if (Number.isInteger(state.after)) query.set("after", String(state.after));
  const text = query.toString();
  return "#" + ROUTE + (text ? "?" + text : "");
}

function defaultAfter(total, size) {
  return Math.max(0, total - size);
}

function clampAfter(after, total, size) {
  return after >= total ? defaultAfter(total, size) : after;
}

function memoryCtx(hashState) {
  return { thread: hashState.thread, after: hashState.after, node: hashState.node };
}

function olderAfter(after, size) {
  return after > 0 ? Math.max(0, after - size) : null;
}

function newerAfter(nextAfter) {
  return Number.isInteger(nextAfter) ? nextAfter : null;
}

function oneLine(text) {
  return String(text).replace(/\s+/g, " ").trim();
}

function firstLine(text) {
  const line = String(text ?? "")
    .split(/\r?\n/)
    .find((part) => part.trim() !== "");
  return line === undefined ? "" : line.trim();
}

function extentSpan(lo, hi, total) {
  if (![lo, hi, total].every(Number.isFinite) || total <= 0 || lo >= hi || lo >= total) return null;
  const width = Math.max(((Math.min(hi, total) - lo) * 100) / total, EXTENT_MIN);
  return { x: Math.min((lo * 100) / total, 100 - width), width };
}

function percent(value) {
  return String(Math.round(value * 100) / 100);
}

function idRange(id) {
  const parts = String(id).split("-").map(Number);
  return { lo: parts[0], hi: parts.length > 1 ? parts[1] : parts[0] };
}

function containsNode(id, target) {
  const outer = idRange(id);
  const inner = idRange(target);
  return id !== target && outer.lo <= inner.lo && inner.hi <= outer.hi;
}

function navTarget(key, rows, at) {
  const row = rows[at];
  if (!row) return null;
  if (key === "ArrowDown") return at + 1 < rows.length ? { focus: at + 1 } : null;
  if (key === "ArrowUp") return at > 0 ? { focus: at - 1 } : null;
  if (key === "Home") return { focus: 0 };
  if (key === "End") return { focus: rows.length - 1 };
  if (key === "ArrowRight") {
    if (!row.expandable) return null;
    if (!row.expanded) return { toggle: true };
    const next = rows[at + 1];
    return next && next.level > row.level ? { focus: at + 1 } : null;
  }
  if (key === "ArrowLeft") {
    if (row.expandable && row.expanded) return { toggle: true };
    for (let up = at - 1; up >= 0; up -= 1) {
      if (rows[up].level < row.level) return { focus: up };
    }
  }
  return null;
}

function el(tag, text, attrs) {
  const node = document.createElement(tag);
  if (text !== undefined && text !== null) node.textContent = text;
  Object.entries(attrs || {}).forEach(([name, value]) => node.setAttribute(name, value));
  return node;
}

function append(parent, ...children) {
  children.forEach((child) => parent.appendChild(child));
  return parent;
}

async function api(path, init) {
  const response = await fetch(path, { credentials: "same-origin", ...init });
  let data = null;
  try {
    data = await response.json();
  } catch (_) {
    data = null;
  }
  if (!response.ok) {
    const message = (data && data.error) || response.statusText || "request failed";
    throw { status: response.status, message };
  }
  return data;
}

function getJson(path, params) {
  return api(path + "?" + new URLSearchParams(params).toString());
}

function renderMarkdown(markdown) {
  return api("/api/render", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ markdown }),
  });
}

function setRendered(host, html) {
  host.innerHTML = html;
}

function emptyState(what, compact, quiet) {
  const box = el("r-empty", null, { role: "status" });
  if (quiet) box.removeAttribute("role");
  if (compact) box.setAttribute("compact", "");
  return append(box, el("span", what, { class: "what" }));
}

function failure(error, retry, compact) {
  const message = (error && error.message) || String(error);
  const box = el("r-error", null, { role: "alert" });
  if (compact) box.setAttribute("compact", "");
  const what = el("span", "Could not load", { class: "what" });
  if (error && error.status) {
    const pill = el("r-pill", null, { tone: "bad", shape: "square", variant: "outline" });
    append(what, append(pill, el("span", String(error.status))));
  }
  const why = el("span", null, { class: "why" });
  why.textContent = message;
  append(box, what, why);
  if (retry) {
    const button = el("button", "Retry", { type: "button", class: "btn" });
    button.addEventListener("click", retry);
    append(box, append(el("span", null, { class: "next" }), button));
  }
  return box;
}

function focusTarget(node) {
  node.setAttribute("tabindex", "-1");
  node.focus();
}

function parentButton(node) {
  return node.closest('[role="treeitem"]').querySelector(":scope > button");
}

function focusRetry(box) {
  const button = box.querySelector("button.btn");
  if (button) button.focus();
}

function delayedSpinner(host, label) {
  const spinner = el("r-spinner", null, { role: "status" });
  spinner.appendChild(el("span", label));
  const timer = setTimeout(() => host.appendChild(spinner), SPINNER_DELAY_MS);
  return () => {
    clearTimeout(timer);
    spinner.remove();
  };
}

function treeButtons(tree) {
  return Array.from(tree.querySelectorAll('li[role="treeitem"] > button'));
}

function visibleRows(tree) {
  return treeButtons(tree)
    .map((button) => button.parentElement)
    .filter((item) => !item.parentElement.closest('li[aria-expanded="false"]'));
}

function rowModel(item) {
  return {
    level: Number(item.getAttribute("aria-level")),
    expandable: item.hasAttribute("aria-expanded"),
    expanded: item.getAttribute("aria-expanded") === "true",
  };
}

function setActive(tree, button) {
  treeButtons(tree).forEach((other) => other.setAttribute("tabindex", other === button ? "0" : "-1"));
}

function onTreeKey(tree, event) {
  const inItem = event.target.closest('[role="treeitem"]');
  const button = event.target.closest('li[role="treeitem"] > button') || (inItem && parentButton(event.target));
  if (!button) return;
  const items = visibleRows(tree);
  const at = items.indexOf(button.parentElement);
  const target = navTarget(event.key, items.map(rowModel), at);
  if (!target) return;
  event.preventDefault();
  if (target.toggle) button.click();
  else items[target.focus].firstElementChild.focus();
}

function svgEl(tag, attrs) {
  const node = document.createElementNS(SVG_NS, tag);
  Object.entries(attrs).forEach(([name, value]) => node.setAttribute(name, value));
  return node;
}

function chevron() {
  const svg = svgEl("svg", { class: "chev", viewBox: "0 0 8 8", "aria-hidden": "true" });
  return append(svg, svgEl("path", { d: "M2.5 1l3.5 3-3.5 3" }));
}

function extentCell(extent) {
  const span = extent && extentSpan(extent.lo, extent.hi, extent.total);
  if (!span) return el("span", null, { class: "extent" });
  const svg = svgEl("svg", { class: "extent", viewBox: "0 0 100 6", preserveAspectRatio: "none", "aria-hidden": "true" });
  return append(
    svg,
    svgEl("rect", { class: "track", y: "2", width: "100", height: "2" }),
    svgEl("rect", { class: "span", x: percent(span.x), width: percent(span.width), height: "6" }),
  );
}

function summaryCell(summary) {
  const cell = el("span", null, { class: "sum" });
  if (summary === null) {
    append(cell, append(el("r-pill", null, { tone: "neutral", shape: "square" }), el("span", "pending")));
  } else if (summary) cell.textContent = oneLine(summary);
  return cell;
}

function treeItem(level, key, label, cells) {
  const item = el("li", null, { role: "treeitem", "aria-expanded": "false", "aria-level": String(level) });
  const button = el("button", null, { type: "button", tabindex: "-1" });
  button.dataset.key = key;
  for (let guide = 1; guide < level; guide += 1) append(button, el("span", null, { class: "g" }));
  append(button, chevron(), el("span", label, { class: "id" }));
  append(item, button);
  append(item, el("span", cells.count === undefined ? null : String(cells.count), { class: "n" }));
  append(item, extentCell(cells.extent), summaryCell(cells.summary));
  append(item, el("ul", null, { role: "group" }));
  return item;
}

function nodeItem(level, node, total) {
  const cells = {
    count: node.hi - node.lo,
    extent: { lo: node.lo, hi: node.hi, total },
    summary: node.summary,
  };
  const item = treeItem(level, node.id, "[" + node.lo + "-" + (node.hi - 1) + "]", cells);
  item.dataset.node = node.id;
  return item;
}

function entryItem(level, entry) {
  const label = "#" + entry.i + " " + entry.role;
  const item = treeItem(level, "e" + entry.i, label, { summary: entry.at + " " + firstLine(entry.text) });
  item.dataset.entry = entry.text;
  return item;
}

function setExpanded(item, expanded) {
  item.setAttribute("aria-expanded", String(expanded));
}

async function fillRendered(row, markdown, retried) {
  const stop = delayedSpinner(row, "Loading text");
  try {
    setRendered(row, (await renderMarkdown(markdown)).html);
    if (retried) {
      const prose = row.firstElementChild;
      if (prose) focusTarget(prose);
      else parentButton(row).focus();
    }
  } catch (error) {
    row.textContent = "";
    const box = failure(error, () => {
      row.textContent = "";
      fillRendered(row, markdown, true);
    }, true);
    row.appendChild(box);
    if (retried) focusRetry(box);
  }
  stop();
}

function rowItem() {
  return el("li", null, { role: "none" });
}

async function openEntry(item) {
  const group = item.querySelector(':scope > ul[role="group"]');
  const row = rowItem();
  append(group, row);
  await fillRendered(row, item.dataset.entry);
}

function zoomRows(zoom) {
  const children = zoom.children.map((child) => ({ row: child.kind === "block" ? "node" : "entry", child }));
  return [{ row: "summary", markdown: zoom.node.summary }, ...children];
}

function clearRows(group) {
  group.querySelectorAll(":scope > li").forEach((row) => row.remove());
}

async function openNode(item, ctx, retried) {
  const group = item.querySelector(':scope > ul[role="group"]');
  clearRows(group);
  const level = Number(item.getAttribute("aria-level")) + 1;
  const total = Number(item.closest("r-tree").dataset.total);
  const mine = generation;
  const stop = delayedSpinner(group, "Loading entries");
  try {
    const zoom = await getJson("/api/memory", { thread: ctx.thread, node: item.dataset.node });
    stop();
    if (mine !== generation) return;
    zoomRows(zoom).forEach((row) => {
      if (row.row === "summary") {
        const summary = rowItem();
        append(group, summary);
        if (row.markdown === null) summary.appendChild(emptyState("summary pending", true));
        else fillRendered(summary, row.markdown);
      } else append(group, row.row === "node" ? nodeItem(level, row.child, total) : entryItem(level, row.child));
    });
    if (retried) {
      const first = group.querySelector(':scope > li[role="treeitem"] > button');
      (first || parentButton(item)).focus();
    }
    history.replaceState(null, "", buildHash({ ...ctx, node: item.dataset.node }));
  } catch (error) {
    stop();
    clearRows(group);
    const row = rowItem();
    append(group, row);
    const box = failure(error, () => openNode(item, ctx, true), true);
    row.appendChild(box);
    if (retried) focusRetry(box);
  }
}

async function toggleItem(item, ctx) {
  if (item.getAttribute("aria-expanded") === "true") return setExpanded(item, false);
  setExpanded(item, true);
  if (item.dataset.loaded) return;
  item.dataset.loaded = "1";
  if (item.dataset.node) await openNode(item, ctx);
  else await openEntry(item);
}

async function expandTo(tree, target, ctx) {
  let scope = tree;
  for (let depth = 0; depth < 64; depth += 1) {
    const items = Array.from(scope.querySelectorAll(':scope li[data-node]'));
    const next = items.find((item) => item.dataset.node === target || containsNode(item.dataset.node, target));
    if (!next) return;
    if (next.getAttribute("aria-expanded") !== "true") await toggleItem(next, ctx);
    if (next.dataset.node === target) return next.firstElementChild.focus();
    scope = next;
  }
}

function buildTree(roots, ctx, total) {
  const tree = el("r-tree", null, { "data-total": String(total) });
  const list = el("ul", null, { role: "tree" });
  append(tree, list);
  roots.forEach((node) => append(list, nodeItem(1, node, total)));
  list.addEventListener("click", (event) => {
    const button = event.target.closest('li[role="treeitem"] > button');
    if (button) toggleItem(button.parentElement, ctx);
  });
  list.addEventListener("keydown", (event) => onTreeKey(list, event));
  list.addEventListener("focusin", (event) => {
    if (event.target.matches('li[role="treeitem"] > button')) setActive(list, event.target);
  });
  const first = treeButtons(list)[0];
  if (first) setActive(list, first);
  return { tree, list };
}

function kv(pairs) {
  const list = el("dl");
  pairs.forEach(([name, value]) => append(list, append(el("div"), el("dt", name), el("dd", value))));
  return append(el("r-kv", null, { two: "" }), list);
}

function threadSelect(threads, current) {
  const select = el("select", null, { id: "thread", "aria-label": "thread" });
  const names = ["main", ...threads.map((thread) => thread.name)];
  if (!names.includes(current)) names.push(current);
  names.forEach((name) => {
    const meta = threads.find((thread) => thread.name === name);
    const note = meta && meta.parked ? " (parked)" : meta && meta.merged ? " (merged)" : "";
    const option = el("option", name + note, { value: name });
    append(select, option);
    option.selected = name === current;
  });
  select.addEventListener("change", () => {
    location.hash = buildHash({ thread: select.value, node: "", after: null });
  });
  return append(el("label"), el("span", "thread ", { class: "meta" }), select);
}

function stateRow(columns, child) {
  const cell = append(el("td", null, { class: "state", colspan: String(columns) }), child);
  return append(el("tr", null, { class: "state" }), cell);
}

const ENTRY_COLUMNS = ["#", "role", "at", "text"];

function entriesTable(page) {
  const head = append(el("tr"), ...ENTRY_COLUMNS.map((name) => el("th", name, name === "#" ? { class: "num" } : {})));
  const body = el("tbody");
  if (page.items.length === 0) {
    append(body, stateRow(ENTRY_COLUMNS.length, emptyState("no entries in this page", true)));
  }
  page.items.forEach((entry) => {
    append(
      body,
      append(
        el("tr"),
        el("td", String(entry.i), { class: "num" }),
        el("td", entry.role),
        el("td", entry.at),
        el("td", firstLine(entry.text), { class: "clip" }),
      ),
    );
  });
  return append(el("r-table", null, { class: "stackable" }), append(el("table"), append(el("thead"), head), body));
}

function pager(page, state) {
  const bar = el("p");
  const older = olderAfter(state.after, PAGE_SIZE);
  const newer = newerAfter(page.next_after);
  if (older !== null) append(bar, el("a", "older", { href: buildHash({ ...state, after: older }) }), document.createTextNode(" "));
  if (newer !== null) append(bar, el("a", "newer", { href: buildHash({ ...state, after: newer }) }));
  return bar;
}

async function loadMemory(state) {
  const [threads, roots] = await Promise.all([
    getJson("/api/memory/threads", {}),
    getJson("/api/memory", { thread: state.thread }),
  ]);
  const after = state.after === null ? defaultAfter(roots.entries, PAGE_SIZE) : clampAfter(state.after, roots.entries, PAGE_SIZE);
  const page = await getJson("/api/memory/entries", { thread: state.thread, after, limit: PAGE_SIZE });
  document.getElementById("store").textContent = threads.store;
  return { threads: threads.threads, roots, page, after };
}

function drawMemory(main, hashState, data) {
  const state = { ...hashState, after: data.after };
  const ctx = memoryCtx(hashState);
  const box = append(el("r-box", null, { shape: "square" }), el("span", "memory", { class: "title" }));
  main.replaceChildren(box);
  append(box, el("div"), threadSelect(data.threads, state.thread));
  append(box, kv([["thread", state.thread], ["entries", String(data.roots.entries)]]));
  const empty = data.roots.entries === 0;
  if (empty) append(box, emptyState("no entries yet"));
  else {
    const { tree, list } = buildTree(data.roots.nodes, ctx, data.roots.entries);
    append(box, tree);
    if (state.node) expandTo(list, state.node, ctx);
  }
  append(box, el("r-rule"), el("span", "recent entries", { class: "title" }));
  append(box, empty ? emptyState("no entries yet", false, true) : entriesTable(data.page), pager(data.page, state));
}

function markCurrentScreen(route) {
  for (const link of document.querySelectorAll("nav a")) {
    if (link.getAttribute("href") === "#" + route) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
}

let generation = 0;

async function show(refocus) {
  const state = parseHash(location.hash);
  const main = document.getElementById("main");
  const mine = ++generation;
  const focusKey = document.activeElement && document.activeElement.dataset.key;
  if (!SCREENS.includes(state.route)) {
    location.replace(buildHash(state));
    return;
  }
  markCurrentScreen(state.route);
  main.replaceChildren();
  const stop = delayedSpinner(main, "Loading memory");
  try {
    const data = await loadMemory(state);
    if (mine !== generation) return;
    stop();
    drawMemory(main, state, data);
    const again = focusKey && main.querySelector('button[data-key="' + focusKey + '"]');
    if (again) again.focus();
    else if (refocus) focusTarget(main);
  } catch (error) {
    if (mine !== generation) return;
    stop();
    const box = failure(error, () => show(true));
    main.replaceChildren(box);
    if (refocus) focusRetry(box);
  }
}

window.addEventListener("hashchange", () => show());
show();
