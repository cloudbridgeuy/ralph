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

function olderAfter(after, size, floor) {
  return after > floor ? Math.max(floor, after - size) : null;
}

function newerAfter(nextAfter, ceiling) {
  return Number.isInteger(nextAfter) && nextAfter < ceiling ? nextAfter : null;
}

function rangeAfter(after, lo, hi) {
  return Number.isInteger(after) && after >= lo && after < hi ? after : lo;
}

function rangeLabel(lo, hi) {
  return "[" + lo + "-" + hi + "]";
}

function pagerScope(node, lo, hi) {
  return node ? lo + "\u2013" + (hi - 1) : String(hi - lo);
}

function pagerLabel(after, count, scope) {
  if (count === 0) return "entries 0 of " + scope;
  return "entries " + after + "\u2013" + (after + count - 1) + " of " + scope;
}

function pagerDirection(event, typing) {
  if (event.ctrlKey || event.metaKey || event.altKey || event.defaultPrevented || typing) return null;
  if (event.key === "[") return "older";
  if (event.key === "]") return "newer";
  return null;
}

function isTyping(target) {
  const tag = target && target.tagName;
  return tag === "INPUT" || tag === "BUTTON" || tag === "SELECT" || tag === "TEXTAREA" || Boolean(target && target.isContentEditable);
}

function crumbParts(thread, path) {
  const parts = [
    { label: "memory", hash: buildHash({ thread: "main" }) },
    { label: thread, hash: buildHash({ thread }) },
    ...path.map((id) => {
      const range = idRange(id);
      return { label: rangeLabel(range.lo, range.hi), hash: buildHash({ thread, node: id }) };
    }),
  ];
  return parts.map((part, at) => (at === parts.length - 1 ? { label: part.label, hash: null } : part));
}

function statLine(stats) {
  const last = stats.last_at ? ", last entry " + stats.last_at : "";
  return stats.entries + " entries, " + stats.blocks + " blocks, " + stats.summarised + " summarised, " + stats.pending + " pending" + last;
}

function pendingLabel(node) {
  return node.folded ? "pending ×" + node.stats.pending : "pending";
}

function detailPairs(node) {
  const range = idRange(node.id);
  return [
    ["range", rangeLabel(range.lo, range.hi)],
    ["entries", String(node.hi - node.lo)],
    ["depth", String(node.stats.depth)],
    ["children", String(node.stats.children)],
    ["summary", node.summary === null ? "pending" : "summarised"],
  ];
}

function rootRows(nodes) {
  return nodes.map((node) =>
    node.leaf && node.entry ? { row: "entry", child: node.entry } : { row: "node", child: node },
  );
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

function failure(error, retry) {
  const status = error && error.status ? error.status + " " : "";
  const message = (error && error.message) || String(error);
  const box = el("r-error", status + message);
  if (retry) {
    box.appendChild(document.createTextNode(" "));
    box.appendChild(el("button", "retry", { type: "button" })).addEventListener("click", retry);
  }
  return box;
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
  const button = event.target.closest('li[role="treeitem"] > button');
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

function pill(text) {
  return append(el("r-pill", null, { tone: "neutral", shape: "square" }), el("span", text));
}

function summaryCell(summary, pending) {
  const cell = el("span", null, { class: "sum" });
  if (summary === null) append(cell, pill(pending));
  else if (summary) cell.textContent = oneLine(summary);
  return cell;
}

function treeItem(level, key, label, cells, expandable) {
  const item = el("li", null, { role: "treeitem", "aria-level": String(level) });
  if (expandable) item.setAttribute("aria-expanded", "false");
  const button = el("button", null, { type: "button", tabindex: "-1" });
  button.dataset.key = key;
  for (let guide = 1; guide < level; guide += 1) append(button, el("span", null, { class: "g" }));
  if (expandable) append(button, chevron());
  append(button, el("span", label, { class: "id" }));
  append(item, button);
  append(item, el("span", cells.count === undefined ? null : String(cells.count), { class: "n" }));
  append(item, extentCell(cells.extent), summaryCell(cells.summary, cells.pending));
  if (expandable) append(item, el("ul", null, { role: "group" }));
  return item;
}

function nodeItem(level, node, total) {
  const cells = {
    count: node.hi - node.lo,
    extent: { lo: node.lo, hi: node.hi, total },
    summary: node.summary,
    pending: pendingLabel(node),
  };
  const item = treeItem(level, node.id, rangeLabel(node.lo, node.hi - 1), cells, !node.folded);
  item.dataset.node = node.id;
  return item;
}

function entryItem(level, entry) {
  const label = "#" + entry.i + " " + entry.role;
  const item = treeItem(level, "e" + entry.i, label, { summary: entry.at + " " + firstLine(entry.text) }, true);
  item.dataset.entry = entry.text;
  return item;
}

function setExpanded(item, expanded) {
  item.setAttribute("aria-expanded", String(expanded));
}

async function fillRendered(row, markdown) {
  const stop = delayedSpinner(row, "Loading text");
  try {
    setRendered(row, (await renderMarkdown(markdown)).html);
  } catch (error) {
    row.textContent = "";
    row.appendChild(failure(error));
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
  return zoom.children.map((child) => ({ row: child.kind === "block" ? "node" : "entry", child }));
}

function drawRow(level, row, total) {
  return row.row === "node" ? nodeItem(level, row.child, total) : entryItem(level, row.child);
}

async function openNode(item, ctx) {
  const group = item.querySelector(':scope > ul[role="group"]');
  const level = Number(item.getAttribute("aria-level")) + 1;
  const total = Number(item.closest("r-tree").dataset.total);
  const stop = delayedSpinner(group, "Loading entries");
  try {
    const zoom = await getJson("/api/memory", { thread: ctx.thread, node: item.dataset.node });
    stop();
    zoomRows(zoom).forEach((row) => append(group, drawRow(level, row, total)));
    return true;
  } catch (error) {
    stop();
    const row = rowItem();
    append(group, row);
    row.appendChild(failure(error));
    return false;
  }
}

async function toggleItem(item, ctx) {
  if (item.getAttribute("aria-expanded") === "true") return setExpanded(item, false);
  setExpanded(item, true);
  if (item.dataset.loaded) return;
  item.dataset.loaded = "1";
  const group = item.querySelector(':scope > ul[role="group"]');
  const ok = item.dataset.node ? await openNode(item, ctx) : (await openEntry(item), true);
  if (!ok) {
    delete item.dataset.loaded;
    group.querySelectorAll(":scope > li").forEach((row) => row.remove());
    setExpanded(item, false);
  }
}

function markSelected(list, item) {
  list.querySelectorAll('li[aria-selected]').forEach((other) => other.removeAttribute("aria-selected"));
  item.setAttribute("aria-selected", "true");
}

async function expandTo(tree, target, ctx) {
  let scope = tree;
  for (let depth = 0; depth < 64; depth += 1) {
    const items = Array.from(scope.querySelectorAll(':scope li[data-node]'));
    const next = items.find((item) => item.dataset.node === target || containsNode(item.dataset.node, target));
    if (!next) return;
    if (next.getAttribute("aria-expanded") === "false") await toggleItem(next, ctx);
    if (next.dataset.node === target) {
      markSelected(tree, next);
      return next.firstElementChild.focus();
    }
    scope = next;
  }
}

function buildTree(roots, ctx, total, onSelect) {
  const tree = el("r-tree", null, { "data-total": String(total) });
  const list = el("ul", null, { role: "tree" });
  append(tree, list);
  rootRows(roots).forEach((row) => append(list, drawRow(1, row, total)));
  list.addEventListener("click", (event) => {
    const button = event.target.closest('li[role="treeitem"] > button');
    if (!button) return;
    const item = button.parentElement;
    if (item.dataset.node && item.getAttribute("aria-selected") !== "true") onSelect(list, item);
    if (item.hasAttribute("aria-expanded")) toggleItem(item, ctx);
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

function threadTabs(threads, current) {
  const nav = el("nav", null, { class: "tabs", "aria-label": "threads" });
  const names = ["main", ...threads.map((thread) => thread.name)];
  if (!names.includes(current)) names.push(current);
  names.forEach((name) => {
    const meta = threads.find((thread) => thread.name === name);
    const note = meta && meta.parked ? "parked" : meta && meta.merged ? "merged" : "";
    const attrs = { href: buildHash({ thread: name }), ...(name === current ? { "aria-current": "page" } : {}) };
    const link = el("a", name, attrs);
    if (note) append(link, pill(note));
    append(nav, link);
  });
  return nav;
}

function crumbs(parts) {
  const nav = el("nav", null, { class: "crumbs", "aria-label": "breadcrumb" });
  parts.forEach((part, at) => {
    if (at > 0) nav.appendChild(document.createTextNode(" / "));
    if (part.hash) append(nav, el("a", part.label, { href: part.hash }));
    else append(nav, el("span", part.label, { "aria-current": "page" }));
  });
  return nav;
}

function stateRow(columns, child) {
  const cell = append(el("td", null, { class: "state", colspan: String(columns) }), child);
  return append(el("tr", null, { class: "state" }), cell);
}

const ENTRY_COLUMNS = ["#", "role", "at", "text"];

function toggleEntry(row, entry, current) {
  const wasOpen = row.classList.contains("open");
  if (current.close) current.close();
  current.close = null;
  if (wasOpen) return;
  const cell = row.lastElementChild;
  const text = el("td", null, { colspan: String(ENTRY_COLUMNS.length) });
  const detail = append(el("tr", null, { class: "state full" }), text);
  row.after(detail);
  fillRendered(text, entry.text);
  cell.textContent = "";
  row.classList.add("open");
  row.setAttribute("aria-selected", "true");
  row.setAttribute("aria-expanded", "true");
  current.close = () => {
    row.classList.remove("open");
    row.removeAttribute("aria-selected");
    row.setAttribute("aria-expanded", "false");
    cell.textContent = firstLine(entry.text);
    detail.remove();
  };
}

function entryRow(entry, current) {
  const row = el("tr", null, { tabindex: "0", "aria-expanded": "false" });
  append(
    row,
    el("td", String(entry.i), { class: "num" }),
    el("td", entry.role),
    el("td", entry.at),
    el("td", firstLine(entry.text), { class: "clip" }),
  );
  row.addEventListener("click", () => toggleEntry(row, entry, current));
  row.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      toggleEntry(row, entry, current);
    }
  });
  return row;
}

function entriesTable(page) {
  const head = append(el("tr"), ...ENTRY_COLUMNS.map((name) => el("th", name, name === "#" ? { class: "num" } : {})));
  const body = el("tbody");
  if (page.items.length === 0) {
    append(body, stateRow(ENTRY_COLUMNS.length, el("r-empty", "no entries in this page")));
  }
  const current = { close: null };
  page.items.forEach((entry) => append(body, entryRow(entry, current)));
  return append(el("r-table", null, { class: "stackable" }), append(el("table"), append(el("thead"), head), body));
}

function pagerButton(name, key, after, state) {
  const attrs = after === null
    ? { class: "btn", "aria-disabled": "true", "data-pager": name }
    : { class: "btn", href: buildHash({ ...state, after }), "data-pager": name };
  return append(el("a", name + " ", attrs), el("kbd", key));
}

function pager(page, state, bounds) {
  const older = olderAfter(state.after, PAGE_SIZE, bounds.lo);
  const newer = newerAfter(page.next_after, bounds.hi);
  return append(
    el("p", null, { class: "pager" }),
    pagerButton("older", "[", older, state),
    pagerButton("newer", "]", newer, state),
    el("span", pagerLabel(state.after, page.items.length, pagerScope(state.node, bounds.lo, bounds.hi))),
  );
}

function onPagerKey(event) {
  const direction = pagerDirection(event, isTyping(event.target));
  const link = direction && document.querySelector('a[data-pager="' + direction + '"][href]');
  if (link) location.hash = link.getAttribute("href");
}

async function loadDetail(state, roots) {
  if (!state.node) {
    const after = state.after === null ? defaultAfter(roots.entries, PAGE_SIZE) : clampAfter(state.after, roots.entries, PAGE_SIZE);
    const page = await getJson("/api/memory/entries", { thread: state.thread, after, limit: PAGE_SIZE });
    return { zoom: null, page, after, lo: 0, hi: roots.entries };
  }
  const zoom = await getJson("/api/memory", { thread: state.thread, node: state.node });
  const { lo, hi } = zoom.node;
  const after = rangeAfter(state.after, lo, hi);
  const limit = Math.min(PAGE_SIZE, hi - after);
  const page = await getJson("/api/memory/entries", { thread: state.thread, after, limit });
  return { zoom, page, after, lo, hi };
}

async function loadMemory(state) {
  const [threads, roots] = await Promise.all([
    getJson("/api/memory/threads", {}),
    getJson("/api/memory", { thread: state.thread }),
  ]);
  const detail = await loadDetail(state, roots);
  document.getElementById("store").textContent = threads.store;
  return { threads: threads.threads, roots, detail };
}

function summaryBlock(node) {
  const host = el("div", null, { class: "summary" });
  if (node.summary === null) append(host, pill(pendingLabel(node)));
  else fillRendered(host, node.summary);
  return host;
}

function drawDetail(pane, state, detail) {
  const { zoom, page, after, lo, hi } = detail;
  pane.replaceChildren(crumbs(crumbParts(state.thread, zoom ? zoom.path : [])));
  if (zoom) append(pane, kv(detailPairs(zoom.node)), summaryBlock(zoom.node));
  append(pane, entriesTable(page), pager(page, { ...state, after }, { lo, hi }));
}

let detailGeneration = 0;

async function selectNode(list, item, screen) {
  markSelected(list, item);
  const state = { thread: screen.ctx.thread, node: item.dataset.node, after: null };
  history.replaceState(null, "", buildHash(state));
  const mine = ++detailGeneration;
  const stop = delayedSpinner(screen.pane, "Loading detail");
  try {
    const detail = await loadDetail(state, screen.roots);
    if (mine !== detailGeneration) return;
    stop();
    drawDetail(screen.pane, state, detail);
  } catch (error) {
    if (mine !== detailGeneration) return;
    stop();
    screen.pane.replaceChildren(failure(error, () => selectNode(list, item, screen)));
  }
}

function drawMemory(main, hashState, data) {
  const ctx = memoryCtx(hashState);
  main.replaceChildren(threadTabs(data.threads, hashState.thread));
  append(main, el("p", statLine(data.roots.stats), { class: "statline" }));
  if (data.roots.entries === 0) {
    append(main, el("r-empty", "no entries yet"));
    return;
  }
  const pane = el("div", null, { class: "detail" });
  const screen = { ctx, pane, roots: data.roots };
  const { tree, list } = buildTree(data.roots.nodes, ctx, data.roots.entries, (from, item) => selectNode(from, item, screen));
  append(main, append(el("div", null, { class: "split" }), tree, pane));
  drawDetail(pane, { thread: hashState.thread, node: hashState.node }, data.detail);
  if (hashState.node) expandTo(list, hashState.node, ctx);
}

function markCurrentScreen(route) {
  for (const link of document.querySelectorAll("nav a")) {
    if (link.getAttribute("href") === "#" + route) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
}

let generation = 0;

async function show() {
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
  } catch (error) {
    if (mine !== generation) return;
    stop();
    main.replaceChildren(failure(error, show));
  }
}

window.addEventListener("hashchange", show);
document.addEventListener("keydown", onPagerKey);
show();
