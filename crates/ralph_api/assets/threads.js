"use strict";

const THREADS_ROUTE = "/threads";
const THREAD_COLUMNS = 8;
const MISSING_CHOICES = 3;
const BUSY_STATUS = 503;
const SERVER_HANDLERS = 8;
const THREAD_HEADS = [
  { label: "name", class: "gname" },
  { label: "state" },
  { label: "fork", class: "num" },
  { label: "created" },
  { label: "active", class: "drop-1" },
  { label: "msgs", class: "num" },
  { label: "workers", class: "num drop-3" },
  { label: "unread", class: "num drop-2" },
];
const THREAD_PILLS = {
  open: { tone: "neutral", icon: "M8 5.5A2.5 2.5 0 1 1 8 10.5A2.5 2.5 0 1 1 8 5.5Z" },
  parked: { tone: "warn", icon: "M6 4V12M10 4V12" },
  merged: { tone: "ok", icon: "M3 8L6.5 11.5L13 4.5" },
  dropped: { tone: "bad", icon: "M4 4L12 12M12 4L4 12", variant: "outline" },
};

function threadPickable(row) {
  return row.kind === "thread";
}

function threadPick(rows, name) {
  const choices = rows.filter(threadPickable);
  if (name === null) return { row: choices[0] || null, missing: false };
  const row = choices.find((choice) => choice.name === name);
  return { row: row || null, missing: !row };
}

function threadParam(hash) {
  const cut = hash.indexOf("?");
  return cut < 0 ? null : new URLSearchParams(hash.slice(cut + 1)).get("thread") || null;
}

function threadsHash(name) {
  return "#" + THREADS_ROUTE + (name === null ? "" : "?" + new URLSearchParams({ thread: name }).toString());
}

function threadsCrumbParts(name) {
  if (name === null) return [{ label: "threads", hash: null }];
  return [{ label: "threads", hash: threadsHash(null) }, { label: name, hash: null }];
}

function threadMove(key, count, at) {
  if (count === 0) return null;
  if (key === "ArrowDown") return at + 1 < count ? at + 1 : null;
  if (key === "ArrowUp") return at > 0 ? at - 1 : null;
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  return null;
}

function threadCommands(name, state) {
  const choices = {
    open: ["ralph chat --thread " + name, "ralph thread pull " + name, "ralph thread merge " + name],
    parked: ["ralph thread revive " + name, "ralph thread drop " + name],
    merged: [],
  };
  return choices[state] || [];
}

function threadWhy(error) {
  if (error && error.status === BUSY_STATUS) {
    return "The server answered busy: " + SERVER_HANDLERS + " requests are in progress.";
  }
  return (error && error.message) || String(error);
}

function threadStats(rows) {
  const count = (state) => rows.filter((row) => row.state === state).length;
  const head = rows.find((row) => row.kind === "head");
  return (
    rows.filter(threadPickable).length + " threads, " +
    count("open") + " open, " +
    count("parked") + " parked, " +
    count("merged") + " merged, " +
    count("dropped") + " dropped, " +
    (head ? head.messages : 0) + " messages on main"
  );
}

function createdParts(iso) {
  const match = AT_SHAPE.exec(String(iso));
  return match ? { date: match[1], time: match[2] } : { date: String(iso), time: "" };
}

function threadText(text) {
  return document.createTextNode(String(text));
}

function threadCount(value) {
  return value ? threadText(value) : el("span", "\u2013", { class: "muted" });
}

function threadPill(state) {
  const look = THREAD_PILLS[state];
  const attrs = { tone: look.tone, shape: "square", ...(look.variant ? { variant: look.variant } : {}) };
  const icon = svgEl("svg", {
    class: "icon",
    "aria-hidden": "true",
    focusable: "false",
    viewBox: "0 0 16 16",
    fill: "none",
    stroke: "currentColor",
    "stroke-width": "1.5",
    "stroke-linecap": "round",
    "stroke-linejoin": "round",
  });
  return append(el("r-pill", null, attrs), append(icon, svgEl("path", { d: look.icon })), el("span", state));
}

function stateCell(row) {
  if (row.kind === "head") return [threadText("head")];
  const nodes = [threadPill(row.state)];
  if (row.age) nodes.push(el("span", row.age, { class: "age" }));
  return nodes;
}

function threadValues(row) {
  return [
    stateCell(row),
    [el("span", "#", { class: "muted" }), threadText(row.base)],
    [threadText(createdParts(row.created).date)],
    [row.active ? threadText(row.active) : threadCount(0)],
    [threadCount(row.messages)],
    [threadCount(row.workers)],
    [row.unread > 0 ? el("span", String(row.unread), { class: "hot" }) : threadCount(0)],
  ];
}

function threadRow(row, selected) {
  const thread = row.kind === "thread";
  const tr = el("tr", null, threadPickable(row) ? { tabindex: "0" } : { class: "quiet" });
  if (selected) tr.setAttribute("aria-selected", "true");
  if (thread) tr.dataset.name = row.name;
  const host = el("span", null, { class: "gn" });
  setRendered(host, row.svg);
  const label = row.kind === "dropped" ? el("span", row.name, { class: "nm" }) : el("span", row.name, { class: "nm strong" });
  if (thread) label.setAttribute("title", row.name);
  append(tr, append(el("td", null, { class: "gname" }), append(host, label)));
  const cells = thread
    ? threadValues(row)
    : [stateCell(row), [], [], [], row.kind === "head" ? [threadCount(row.messages)] : [], [], []];
  cells.forEach((nodes, at) => {
    const column = THREAD_HEADS[at + 1];
    const cell = el("td", null, column.class ? { class: column.class } : {});
    cell.classList.toggle("muted", column.label === "active");
    append(tr, append(cell, ...nodes));
  });
  return tr;
}

function detailBox(name) {
  const box = el("r-box");
  return append(box, append(el("span", name, { class: "title" }), el("span", "thread", { class: "sub" })));
}

function detailList(pairs) {
  const list = el("dl");
  pairs.forEach(([name, nodes]) => append(list, append(el("div"), el("dt", name), append(el("dd"), ...nodes))));
  return append(el("r-kv", null, { two: "" }), list);
}

function createdValue(iso) {
  const { date, time } = createdParts(iso);
  if (!time) return [threadText(date)];
  return [el("span", date, { class: "muted" }), threadText(" " + time), el("span", "Z", { class: "muted" })];
}

function ruleBar(label) {
  return append(el("r-rule"), el("b", label));
}

function threadDetail(row, head) {
  const box = detailBox(row.name);
  const fork = [
    threadText("#" + row.base + " "),
    el("span", "of " + head.messages + " on main", { class: "muted" }),
    extentCell({ lo: row.base, hi: head.messages, total: head.messages }),
  ];
  append(
    box,
    el("span", row.messages + " messages", { class: "aside" }),
    detailList([
      ["state", [threadPill(row.state)]],
      ["fork", fork],
      ["created", createdValue(row.created)],
      ["active", [threadText(row.active || "\u2013")]],
      ["messages", [threadText(row.messages)]],
      ["workers", [threadText(row.workers)]],
      ["unread", [threadText(row.unread)]],
    ]),
    ruleBar("memory"),
    append(
      el("div", null, { class: "pane-link" }),
      el("a", "Open the memory of " + row.name + " \u2192", { href: buildHash({ thread: row.name }) }),
      append(el("span", null, { class: "muted" }), el("kbd", "enter")),
    ),
  );
  const commands = threadCommands(row.name, row.state);
  if (commands.length === 0) return box;
  const chips = el("div", null, { class: "cmds" });
  commands.forEach((text) => append(chips, el("span", text, { class: "cmd" })));
  return append(box, ruleBar("in the terminal"), chips);
}

function failedPill(text) {
  return append(el("r-pill", null, { tone: "bad", shape: "square", variant: "outline" }), el("span", text));
}

function threadsFailure(error, retry) {
  const box = el("r-error", null, { role: "alert" });
  const what = el("span", null, { class: "what" });
  if (error && error.status) append(what, failedPill(String(error.status)));
  append(what, el("span", "The threads screen did not load."));
  const button = el("button", null, { type: "button", class: "btn" });
  append(button, el("kbd", "r"), threadText("Retry"));
  button.addEventListener("click", retry);
  return append(
    box,
    what,
    el("span", threadWhy(error), { class: "why" }),
    append(el("span", null, { class: "next" }), button),
  );
}

function threadsMissing(name, choices) {
  const what = append(
    el("span", null, { class: "what" }),
    failedPill("404"),
    threadText("Thread "),
    el("strong", name),
    threadText(" was not found."),
  );
  const why = "The link is old, or the thread was dropped. The table stays in view and no row is selected.";
  const next = append(el("span", null, { class: "next" }), el("span", "Open a thread that exists:"));
  choices.slice(0, MISSING_CHOICES).forEach((choice) => {
    append(next, threadText(" "), el("a", choice.name, { href: threadsHash(choice.name) }));
  });
  const box = append(el("r-error", null, { role: "alert" }), what, el("span", why, { class: "why" }), next);
  return append(detailBox(name), box);
}

function threadsEmpty(store) {
  const box = emptyState("no threads in ");
  append(box.querySelector(".what"), el("code", store, { class: "cmd-inline" }));
  const next = append(
    el("span", null, { class: "next" }),
    el("span", "Open a thread from main:"),
    threadText(" "),
    el("span", "ralph chat --thread <name>", { class: "cmd" }),
  );
  return append(box, next);
}

function keyHints() {
  const hint = (keys, label) => {
    const part = append(el("span"), ...keys.map((key) => el("kbd", key)));
    return label === undefined ? part : append(part, threadText(label));
  };
  const keys = append(
    el("span", null, { class: "keys" }),
    hint(["\u2191\u2193"], "move"),
    hint(["home", "end"]),
    hint(["enter"], "memory"),
  );
  return append(el("span", null, { class: "aside hide-narrow" }), keys);
}

function pickThread(screen, row) {
  screen.trs.forEach((tr) => tr.removeAttribute("aria-selected"));
  screen.trs.get(row.name).setAttribute("aria-selected", "true");
  history.replaceState(null, "", threadsHash(row.name));
  const trail = crumbs(threadsCrumbParts(row.name));
  screen.crumbs.replaceWith(trail);
  screen.crumbs = trail;
  const pane = threadDetail(row, screen.head);
  screen.pane.replaceWith(pane);
  screen.pane = pane;
}

function drawThreads(main, data, wanted) {
  const selection = threadPick(data.rows, wanted);
  const choices = data.rows.filter(threadPickable);
  const head = data.rows.find((row) => row.kind === "head");
  const screen = { head, trs: new Map(), pane: null, crumbs: null };
  screen.crumbs = crumbs(threadsCrumbParts(selection.row ? selection.row.name : wanted));
  const body = el("tbody");
  data.rows.forEach((row) => {
    const tr = threadRow(row, selection.row === row);
    if (threadPickable(row)) {
      screen.trs.set(row.name, tr);
      tr.addEventListener("click", () => pickThread(screen, row));
    }
    append(body, tr);
  });
  if (choices.length === 0) append(body, stateRow(THREAD_COLUMNS, threadsEmpty(data.store)));
  const heads = THREAD_HEADS.map((column) => el("th", column.label, column.class ? { class: column.class } : {}));
  const table = append(el("table", null, { "aria-label": "threads" }), append(el("thead"), append(el("tr"), ...heads)), body);
  const box = el("r-box", null, { flush: "" });
  append(
    box,
    append(el("span", "threads", { class: "title" }), el("span", data.store, { class: "sub" })),
    choices.length === 0 ? el("span", "0 threads", { class: "aside" }) : keyHints(),
    append(el("r-table", null, { class: "threads" }), table),
  );
  main.replaceChildren(screen.crumbs, el("p", threadStats(data.rows), { class: "statline" }));
  if (choices.length === 0) {
    append(main, box);
    return;
  }
  screen.pane = selection.missing ? threadsMissing(wanted, choices) : threadDetail(selection.row, head);
  append(main, append(el("div", null, { class: "split master-wide" }), box, screen.pane));
}

async function loadThreads() {
  const data = await getJson("/api/threads", {});
  document.getElementById("store").textContent = data.store;
  return data;
}

async function showThreads(main, mine, refocus) {
  const stop = delayedSpinner(main, "Loading threads\u2026");
  try {
    const data = await loadThreads();
    if (mine !== generation) return stop();
    stop();
    drawThreads(main, data, threadParam(location.hash));
    if (refocus) focusTarget(main);
  } catch (error) {
    if (mine !== generation) return stop();
    stop();
    const box = threadsFailure(error, () => show(true));
    main.replaceChildren(box);
    if (refocus) focusRetry(box);
  }
}

function onThreadsKey(event) {
  if (parseHash(location.hash).route !== THREADS_ROUTE) return;
  if (event.ctrlKey || event.metaKey || event.altKey || event.defaultPrevented) return;
  if (event.key === "r" && (!isTyping(event.target) || event.target.closest("r-error"))) {
    const retry = document.querySelector("main > r-error button.btn");
    if (retry) retry.click();
    return;
  }
  if (isTyping(event.target) || event.target.closest("a")) return;
  const rows = Array.from(document.querySelectorAll("r-table.threads tbody tr[data-name]"));
  const at = rows.findIndex((row) => row.getAttribute("aria-selected") === "true");
  if (event.key === "Enter" || event.key === " ") {
    const focused = event.target.closest("tr[data-name]");
    const row = focused || (event.key === "Enter" ? rows[at] : null);
    if (!row) return;
    event.preventDefault();
    if (row !== rows[at]) row.click();
    if (event.key === "Enter") location.hash = buildHash({ thread: row.dataset.name });
    return;
  }
  const next = threadMove(event.key, rows.length, at);
  if (next === null) return;
  event.preventDefault();
  rows[next].click();
  rows[next].focus();
}

document.addEventListener("keydown", onThreadsKey);
