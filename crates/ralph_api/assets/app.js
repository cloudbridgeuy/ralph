"use strict";

const SCREENS = ["/memory"];
const ROUTE = SCREENS[0];
const PAGE_SIZE = 20;
const PREVIEW = 80;
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

function preview(text, max) {
  const line = String(text).replace(/\s+/g, " ").trim();
  return line.length > max ? line.slice(0, max - 1) + "…" : line;
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

function treeItem(level, key, label, meta) {
  const item = el("li", null, { role: "treeitem", "aria-expanded": "false", "aria-level": String(level) });
  const button = el("button", label, { type: "button", tabindex: "-1" });
  button.dataset.key = key;
  append(item, button);
  if (meta) append(item, el("span", meta, { class: "meta" }));
  append(item, el("ul", null, { role: "group" }));
  return item;
}

function nodeItem(level, node) {
  const meta = node.summary === null ? "summary pending" : preview(node.summary, PREVIEW);
  const item = treeItem(level, node.id, "[" + node.lo + "-" + (node.hi - 1) + "]", meta);
  item.dataset.node = node.id;
  return item;
}

function entryItem(level, entry) {
  const label = "#" + entry.i + " " + entry.role;
  const item = treeItem(level, "e" + entry.i, label, entry.at + " " + preview(entry.text, PREVIEW));
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
  const children = zoom.children.map((child) => ({ row: child.kind === "block" ? "node" : "entry", child }));
  return [{ row: "summary", markdown: zoom.node.summary }, ...children];
}

async function openNode(item, ctx) {
  const group = item.querySelector(':scope > ul[role="group"]');
  const level = Number(item.getAttribute("aria-level")) + 1;
  const stop = delayedSpinner(group, "Loading entries");
  try {
    const zoom = await getJson("/api/memory", { thread: ctx.thread, node: item.dataset.node });
    stop();
    zoomRows(zoom).forEach((row) => {
      if (row.row === "summary") {
        const summary = rowItem();
        append(group, summary);
        if (row.markdown === null) summary.appendChild(el("r-empty", "summary pending"));
        else fillRendered(summary, row.markdown);
      } else append(group, row.row === "node" ? nodeItem(level, row.child) : entryItem(level, row.child));
    });
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
    return;
  }
  if (item.dataset.node) history.replaceState(null, "", buildHash({ ...ctx, node: item.dataset.node }));
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

function buildTree(roots, ctx) {
  const tree = el("r-tree");
  const list = el("ul", null, { role: "tree" });
  append(tree, list);
  roots.forEach((node) => append(list, nodeItem(1, node)));
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

function kv(rows, variant) {
  const list = el("dl");
  rows.forEach(([name, value, source]) => {
    const line = append(el("div"), el("dt", name), el("dd", value));
    if (source !== undefined) {
      append(line, el("dd", source, { class: source === "default" ? "src" : "src set" }));
    }
    append(list, line);
  });
  return append(el("r-kv", null, variant ? { [variant]: "" } : {}), list);
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

function entriesTable(page) {
  if (page.items.length === 0) return el("r-empty", "no entries in this page");
  const head = append(el("tr"), ...["#", "role", "at", "text"].map((name) => el("th", name)));
  const body = el("tbody");
  page.items.forEach((entry) => {
    append(
      body,
      append(
        el("tr"),
        el("td", String(entry.i)),
        el("td", entry.role),
        el("td", entry.at),
        el("td", preview(entry.text, PREVIEW)),
      ),
    );
  });
  return append(el("r-table"), append(el("table"), append(el("thead"), head), body));
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
  return { threads: threads.threads, roots, page, after };
}

function drawMemory(main, hashState, data) {
  const state = { ...hashState, after: data.after };
  const ctx = memoryCtx(hashState);
  const box = append(el("r-box", null, { shape: "square" }), el("span", "memory", { class: "title" }));
  main.replaceChildren(box);
  append(box, el("div"), threadSelect(data.threads, state.thread));
  append(box, kv([["thread", state.thread], ["entries", String(data.roots.entries)]], "two"));
  const empty = data.roots.entries === 0;
  if (empty) append(box, el("r-empty", "no entries yet"));
  else {
    const { tree, list } = buildTree(data.roots.nodes, ctx);
    append(box, tree);
    if (state.node) expandTo(list, state.node, ctx);
  }
  append(box, el("r-rule"), el("span", "recent entries", { class: "title" }));
  append(box, empty ? el("r-empty", "no entries yet") : entriesTable(data.page), pager(data.page, state));
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
show();
