"use strict";
const D = JSON.parse(document.getElementById("opb-data").textContent),
  $ = (id) => document.getElementById(id);
const TOP = 52,
  ROW = 38;
let rows = [],
  columns = [],
  selection = { r: 0, id: D.columns[0] },
  rangeSelection = null,
  drag = null,
  ignoreClick = false,
  width = 1,
  cw = 1;
const def = (cell) => (cell ? D.definitions[cell.case_hash] : null),
  group = (id) => id.split("-")[1];
function option(select, value, label) {
  const o = document.createElement("option");
  o.value = value;
  o.textContent = label;
  select.append(o);
}
function time(ns) {
  if (ns == null) return "—";
  const units = [
    [1e9, "s"],
    [1e6, "ms"],
    [1e3, "µs"],
    [1, "ns"],
  ];
  const [scale, unit] = units.find(([s]) => ns >= s) || units[3];
  return (
    (ns / scale).toLocaleString(undefined, { maximumSignificantDigits: 4 }) +
    " " +
    unit +
    "/op"
  );
}
function relativeEffect(n) {
  if (!(n > 0) || !Number.isFinite(n)) return "—";
  if (n === 1) return "1×";
  const magnitude = n < 1 ? 1 / n : n;
  const text = magnitude.toLocaleString(undefined, {
    maximumSignificantDigits: 3,
  });
  return text === "1" ? "≈1×" : (n < 1 ? "+" : "−") + text + "×";
}
function run() {
  return D.runs[+$("run").value];
}
function cellAt(row, id) {
  return row?.cells?.[id] || null;
}
function metric(row, id, mode = $("mode").value) {
  const c = cellAt(row, id);
  if (!c || c.status !== "passed" || !c.timing)
    return { value: null, reason: c?.status || "Not recorded in this run" };
  if (mode === "absolute") return { value: c.timing.mean };
  if (mode === "run") {
    const result = D.comparisons[c.comparisons[+$("compare").value]];
    return {
      value: result.ratio ?? null,
      reason: result.reasons.join("; "),
      comparison: result,
    };
  }
  const reference = rows[+$("reference").value],
    b = cellAt(reference, id);
  if (!b || b.status !== "passed" || !b.timing)
    return { value: null, reason: "Reference has no verified measurement" };
  if (c.case_hash !== b.case_hash)
    return { value: null, reason: "Case parameters / timer boundary differ" };
  return { value: c.timing.mean / b.timing.mean, reference: b.timing };
}
function factsItem(title, value) {
  const box = document.createElement("div"),
    label = document.createElement("small"),
    v = document.createElement("strong");
  label.textContent = title;
  v.textContent = value;
  box.append(label, v);
  $("facts").append(box);
}
function link(parent, title, url) {
  if (!url) return;
  const a = document.createElement("a");
  a.textContent = title;
  a.href = url;
  a.target = "_blank";
  a.rel = "noopener";
  parent.append(a);
  parent.append(document.createTextNode(" "));
}
function selectRange(r, anchor, focus) {
  const a = columns.indexOf(anchor),
    b = columns.indexOf(focus);
  rangeSelection = {
    r,
    anchor,
    focus,
    ids: columns.slice(Math.min(a, b), Math.max(a, b) + 1),
  };
  selection = { r, id: anchor };
}
function rangeSummary() {
  $("rangeSummary").hidden = !rangeSelection;
  $("backendLabel").hidden =
    $("mode").value === "run" ||
    ($("mode").value === "absolute" && !rangeSelection);
  if (!rangeSelection) return;
  const row = rows[rangeSelection.r],
    ids = rangeSelection.ids;
  const times = [],
    ratios = [];
  let timeouts = 0;
  const mode = $("mode").value === "run" ? "run" : "backend";
  for (const id of ids) {
    const c = cellAt(row, id);
    if (c?.status === "timeout") timeouts++;
    if (c?.status === "passed" && c.timing) times.push(c.timing.mean);
    const ratio = metric(row, id, mode).value;
    if (ratio > 0 && Number.isFinite(ratio)) ratios.push(ratio);
  }
  $("rangeTitle").textContent =
    row.label + " · " + ids.length + " selected cases";
  $("rangeCounts").textContent =
    "Time: " +
    times.length +
    " included, " +
    (ids.length - times.length) +
    " excluded (" +
    timeouts +
    " timeout). Effect: " +
    ratios.length +
    " comparable, " +
    (ids.length - ratios.length) +
    " excluded.";
  $("rangeReference").textContent =
    "Effect relative to " +
    (mode === "run"
      ? $("compare").selectedOptions[0].textContent
      : rows[+$("reference").value].label) +
    ".";
  $("rangeStats").replaceChildren();
  for (const [label, values, format] of [
    ["Time per operation", times, time],
    ["Relative effect", ratios, relativeEffect],
  ]) {
    const tr = document.createElement("tr"),
      th = document.createElement("th");
    th.scope = "row";
    th.textContent = label;
    tr.append(th);
    const stats = values.length
      ? [
          Math.min(...values),
          Math.max(...values),
          values.reduce((mean, value, i) => mean + (value - mean) / (i + 1), 0),
        ]
      : [null, null, null];
    for (const value of stats) {
      const td = document.createElement("td");
      td.textContent = value == null ? "—" : format(value);
      tr.append(td);
    }
    $("rangeStats").append(tr);
  }
}
function rowSet() {
  const r = run(),
    result = [];
  for (const backend of D.backends) {
    const found = r.rows.filter((row) => row.backend === backend);
    result.push(
      ...(found.length
        ? found.map((row) => ({
            ...row,
            label: found.length === 1 ? backend : row.label,
          }))
        : [{ backend, label: backend, cells: {}, missing: true }]),
    );
  }
  return result;
}
function refreshReferences() {
  const old = $("reference").selectedOptions[0]?.textContent;
  $("reference").replaceChildren();
  rows = rowSet();
  rows.forEach((r, i) => option($("reference"), i, r.label));
  let at = rows.findIndex((r) => r.label === old && !r.missing);
  if (at < 0) at = rows.findIndex((r) => r.backend === "fst-lib" && !r.missing);
  if (at < 0) at = rows.findIndex((r) => !r.missing);
  $("reference").value = String(Math.max(0, at));
}
function describe() {
  const row = rows[selection.r],
    c = cellAt(row, selection.id),
    m = metric(row, selection.id),
    caseDef = def(c);
  $("cellTitle").textContent = (row?.label || "") + " · " + selection.id;
  $("facts").replaceChildren();
  $("samples").replaceChildren();
  $("sampleCaption").textContent = "";
  $("links").replaceChildren();
  if (!c) {
    $("explanation").textContent =
      "Not recorded in this run. This is not a zero-duration operation.";
    $("cellContext").textContent = "";
    return;
  }
  factsItem("State", c.status);
  factsItem("Mean", c.timing ? time(c.timing.mean) : "—");
  factsItem(
    $("mode").value === "absolute" ? "Process wall time" : "Relative effect",
    $("mode").value === "absolute"
      ? c.measurement.wall_seconds?.toFixed(3) + " s"
      : relativeEffect(m.value),
  );
  if (c.timing) {
    factsItem("Sample spread (CV)", (100 * c.timing.cv).toFixed(1) + "%");
    factsItem(
      (100 * c.timing.confidence).toFixed(0) + "% mean CI",
      time(c.timing.lower) + " – " + time(c.timing.upper),
    );
  }
  let message =
    m.reason ||
    ($("mode").value === "absolute"
      ? "Operation mean; process wall time also includes setup, warmup and analysis."
      : "+5× means five times faster; −5× means five times slower than the reference. 1× means equal time.");
  if (m.comparison) {
    const p = m.comparison;
    message += " " + p.conditions.join("; ");
    if (p.ratio != null)
      message +=
        " " +
        (p.ci_overlap
          ? "Mean confidence intervals overlap."
          : "Mean confidence intervals do not overlap.") +
        " This alone does not establish a code regression.";
  }
  if (m.reference)
    message +=
      " Reference mean: " +
      time(m.reference.mean) +
      ", CI " +
      time(m.reference.lower) +
      " – " +
      time(m.reference.upper) +
      ".";
  $("explanation").textContent = message;
  if (c.timing) {
    const max = Math.max(...c.timing.samples);
    $("sampleCaption").textContent =
      c.timing.samples.length + " samples · time per operation";
    c.timing.samples.forEach((n, i) => {
      const sample = document.createElement("div");
      sample.className = "sample";
      sample.title = "Sample " + (i + 1) + ": " + time(n);
      const label = document.createElement("span");
      label.className = "sample-value";
      label.textContent = time(n);
      const bar = document.createElement("span");
      bar.className = "sample-bar";
      bar.style.height = Math.max(2, (70 * n) / max) + "px";
      const index = document.createElement("small");
      index.textContent = String(i + 1);
      sample.append(label, bar, index);
      $("samples").append(sample);
    });
  }
  link($("links"), "Original run.json · job " + c.job, run().href);
  link($("links"), "Measurement log", c.log);
  link($("links"), "Raw directory", c.raw_directory);
  if (c.timing)
    for (const name of ["sample", "estimates", "benchmark"])
      link($("links"), name + ".json", c.timing.raw_prefix + name + ".json");
  $("cellContext").textContent = JSON.stringify(
    {
      case: caseDef,
      input_summary: D.inputs[c.input_summary],
      input_sha256: c.input_hash,
      artifact: D.artifacts[c.artifact],
      measurement: c.measurement,
      verification: c.verification,
      data_error: c.data_error,
      link_error: c.link_error,
      samples_ns_per_operation: c.timing?.samples,
    },
    null,
    2,
  );
}
function mix(a, b, t) {
  return a.map((v, i) => Math.round(v + (b[i] - v) * t));
}
function colour(value, min, max) {
  const absolute = $("mode").value === "absolute";
  const stops = absolute
    ? [
        [34, 197, 94],
        [250, 204, 21],
        [239, 68, 68],
      ]
    : [
        [250, 204, 21],
        [34, 197, 94],
        [239, 68, 68],
      ];
  const position = absolute
    ? max === min
      ? 0.5
      : (Math.log10(value) - min) / (max - min)
    : (Math.log2(value) + 6) / 12;
  const t = Math.max(0, Math.min(1, position));
  return (
    "rgb(" +
    (t < 0.5
      ? mix(stops[0], stops[1], t * 2)
      : mix(stops[1], stops[2], t * 2 - 1)
    ).join(",") +
    ")"
  );
}
function draw() {
  const canvas = $("map"),
    viewport = $("scroll");
  width = Math.max(1, viewport.clientWidth * +$("zoom").value);
  cw = width / Math.max(1, columns.length);
  const height = TOP + ROW * rows.length;
  const scale = Math.min(window.devicePixelRatio || 1, 16000 / width);
  canvas.width = Math.ceil(width * scale);
  canvas.height = Math.ceil(height * scale);
  canvas.style.width = width + "px";
  canvas.style.height = height + "px";
  const ctx = canvas.getContext("2d");
  ctx.scale(scale, scale);
  ctx.fillStyle = "#111927";
  ctx.fillRect(0, 0, width, height);
  const values = rows
    .flatMap((row) => columns.map((id) => metric(row, id).value))
    .filter((v) => v != null);
  const min = values.length ? Math.log10(Math.min(...values)) : 0,
    max = values.length ? Math.log10(Math.max(...values)) : 1;
  $("scale").classList.toggle("absolute", $("mode").value === "absolute");
  $("scaleText").textContent =
    $("mode").value === "absolute"
      ? values.length
        ? time(10 ** min)
        : "No numeric cells"
      : "+64× or faster";
  $("scaleEnd").textContent =
    $("mode").value === "absolute"
      ? values.length
        ? time(10 ** max) + " · log scale"
        : ""
      : "−64× or slower · 1× at green · log₂ scale";
  let start = 0;
  while (start < columns.length) {
    let end = start + 1;
    while (
      end < columns.length &&
      group(columns[end]) === group(columns[start])
    )
      end++;
    ctx.fillStyle = start % 2 ? "#243650" : "#1b2a41";
    ctx.fillRect(start * cw, 0, (end - start) * cw - 0.5, TOP);
    if ((end - start) * cw > 30) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(start * cw + 2, 0, (end - start) * cw - 4, TOP);
      ctx.clip();
      ctx.fillStyle = "#c0cede";
      ctx.font = "10px system-ui";
      ctx.fillText(group(columns[start]), start * cw + 4, 18);
      ctx.restore();
    }
    start = end;
  }
  ctx.font = "10px ui-monospace,monospace";
  ctx.textAlign = "center";
  ctx.fillStyle = "#e4e8f0";
  columns.forEach((id, i) => {
    const number = id.split("-").at(-1);
    if (ctx.measureText(number).width + 6 <= cw)
      ctx.fillText(number, (i + 0.5) * cw, TOP - 10);
  });
  ctx.textAlign = "start";
  rows.forEach((row, r) =>
    columns.forEach((id, i) => {
      const c = cellAt(row, id),
        v = metric(row, id).value,
        x = i * cw,
        y = TOP + r * ROW;
      ctx.fillStyle =
        v != null
          ? colour(v, min, max)
          : c?.status === "timeout"
            ? "#38bdf8"
            : c &&
                [
                  "measurement failed",
                  "verification failed",
                  "invalid raw",
                  "run error",
                ].includes(c.status)
              ? "#b38cff"
              : "#263143";
      ctx.fillRect(x, y, Math.max(0.5, cw - 0.4), ROW - 1);
      if (
        v == null &&
        (!c ||
          ![
            "timeout",
            "measurement failed",
            "verification failed",
            "invalid raw",
            "run error",
          ].includes(c.status))
      ) {
        ctx.fillStyle = "#4b576c";
        ctx.fillRect(x, y + ROW / 2, Math.max(0.5, cw - 0.4), 2);
      }
      if (c?.timing?.cv >= 0.2) {
        ctx.fillStyle = "#fff";
        ctx.fillRect(x, y, Math.max(0.5, cw - 0.4), 2);
      }
      if (!rangeSelection && r === selection.r && id === selection.id) {
        ctx.strokeStyle = "#fff";
        ctx.lineWidth = 2;
        ctx.strokeRect(x + 0.5, y + 1, Math.max(1, cw - 1), ROW - 3);
      }
    }),
  );
  if (rangeSelection) {
    const first = columns.indexOf(rangeSelection.ids[0]);
    const x = first * cw,
      y = TOP + rangeSelection.r * ROW;
    const size = rangeSelection.ids.length * cw;
    ctx.fillStyle = "rgba(255,255,255,0.12)";
    ctx.fillRect(x, y, size, ROW - 1);
    ctx.strokeStyle = "#fff";
    ctx.lineWidth = 2;
    ctx.strokeRect(x + 0.5, y + 1, Math.max(1, size - 1), ROW - 3);
  }
  $("zoomText").textContent =
    (+$("zoom").value).toFixed(2).replace(/\.00$/, "") + "×";
  rangeSummary();
}
function render() {
  const previousColumns = columns;
  rows = rowSet();
  const first = (id) => rows.map((r) => cellAt(r, id)).find(Boolean);
  columns = D.columns.filter((id) => {
    const c = def(first(id));
    return (
      (!$("group").value || group(id) === $("group").value) &&
      (!$("profile").value || c?.fixture === $("profile").value) &&
      id.toLowerCase().includes($("search").value.toLowerCase())
    );
  });
  const order = $("order").value;
  if (order !== "catalog") {
    const score = (id) =>
      Math.max(
        -Infinity,
        ...rows.map((r) => {
          const c = cellAt(r, id);
          return order === "ratio"
            ? (metric(r, id).value ?? -Infinity)
            : order === "spread"
              ? (c?.timing?.cv ?? -Infinity)
              : c?.status === "passed"
                ? c.timing.mean
                : -Infinity;
        }),
      );
    const scores = new Map(columns.map((id) => [id, score(id)]));
    columns.sort((a, b) => scores.get(b) - scores.get(a));
  }
  if (
    previousColumns.length !== columns.length ||
    previousColumns.some((id, i) => id !== columns[i])
  )
    rangeSelection = null;
  if (!columns.includes(selection.id))
    selection.id = columns[0] || D.columns[0];
  selection.r = Math.min(selection.r, rows.length - 1);
  $("labels").replaceChildren();
  rows.forEach((row) => {
    const label = document.createElement("div");
    label.textContent = row.label + (row.missing ? " · absent" : "");
    label.title = row.config?.sdk_home || row.backend;
    $("labels").append(label);
  });
  const all = run().rows.flatMap((r) => Object.values(r.cells)),
    counts = {};
  all.forEach((c) => (counts[c.status] = (counts[c.status] || 0) + 1));
  $("summary").textContent =
    all.length +
    " saved jobs · " +
    Object.entries(counts)
      .map(([k, v]) => v + " " + k)
      .join(" · ") +
    (run().meta.finished_at ? "" : " · UNFINISHED: provisional records");
  $("range").textContent =
    columns.length +
    " / " +
    D.columns.length +
    " case columns · " +
    (columns[0] || "—") +
    " → " +
    (columns.at(-1) || "—") +
    ". Drag across a row to select cases; Shift + left/right arrow extends a selection; Escape clears it. Scroll to pan when zoomed; Ctrl + wheel to zoom. Group headers focus a group.";
  $("compareLabel").hidden = $("mode").value !== "run";
  $("runContext").textContent = JSON.stringify(
    { metadata: run().meta, configurations: run().rows.map((r) => r.config) },
    null,
    2,
  );
  $("runLinks").replaceChildren();
  link($("runLinks"), "Original manifest", run().href);
  link($("runLinks"), "Experiment context", run().context);
  draw();
  describe();
}
function locate(event) {
  const rect = $("map").getBoundingClientRect();
  return {
    i: Math.max(
      0,
      Math.min(
        columns.length - 1,
        Math.floor((event.clientX - rect.left) / cw),
      ),
    ),
    r: Math.floor((event.clientY - rect.top - TOP) / ROW),
  };
}
$("map").addEventListener("pointerdown", (e) => {
  ignoreClick = false;
  const p = locate(e);
  if (
    e.button !== 0 ||
    e.pointerType === "touch" ||
    p.r < 0 ||
    p.r >= rows.length ||
    !columns[p.i]
  )
    return;
  drag = {
    pointer: e.pointerId,
    r: p.r,
    anchor: columns[p.i],
    previousRange: rangeSelection,
    previousSelection: selection,
    moved: false,
  };
  $("map").focus({ preventScroll: true });
  e.preventDefault();
  $("map").setPointerCapture(e.pointerId);
});
$("map").addEventListener("pointermove", (e) => {
  if (!drag || drag.pointer !== e.pointerId) return;
  const id = columns[locate(e).i];
  if (!id || (!drag.moved && id === drag.anchor)) return;
  drag.moved = true;
  selectRange(drag.r, drag.anchor, id);
  draw();
});
$("map").addEventListener("pointerup", (e) => {
  if (!drag || drag.pointer !== e.pointerId) return;
  ignoreClick = drag.moved;
  if (drag.moved) describe();
  drag = null;
  $("map").releasePointerCapture(e.pointerId);
  $("map").focus({ preventScroll: true });
});
function cancelDrag(e) {
  if (!drag || drag.pointer !== e.pointerId) return;
  rangeSelection = drag.previousRange;
  selection = drag.previousSelection;
  drag = null;
  ignoreClick = true;
  draw();
}
$("map").addEventListener("pointercancel", cancelDrag);
$("map").addEventListener("lostpointercapture", cancelDrag);
$("map").addEventListener("mousemove", (e) => {
  const p = locate(e),
    id = columns[p.i];
  if (p.r < 0 || p.r >= rows.length || !id) {
    $("tip").textContent = id ? group(id) + " · click header to focus" : "";
    return;
  }
  const c = cellAt(rows[p.r], id),
    m = metric(rows[p.r], id);
  $("tip").textContent =
    rows[p.r].label +
    " · " +
    id +
    " · " +
    (c?.status || "not recorded") +
    " · " +
    (c ? def(c).fixture + "/" + def(c).boundary : "") +
    "\n" +
    (c?.timing ? time(c.timing.mean) : "—") +
    " · " +
    ($("mode").value === "absolute" ? "absolute" : relativeEffect(m.value)) +
    (m.reason ? " · " + m.reason : "");
});
$("map").addEventListener("click", (e) => {
  if (ignoreClick) {
    ignoreClick = false;
    return;
  }
  const p = locate(e);
  if (!columns[p.i]) return;
  if (p.r < 0) {
    $("group").value = group(columns[p.i]);
    $("zoom").value = "1";
    $("scroll").scrollLeft = 0;
    render();
    return;
  }
  if (p.r >= rows.length) return;
  rangeSelection = null;
  selection = { r: p.r, id: columns[p.i] };
  draw();
  describe();
});
$("map").addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    if (drag) {
      const pointer = drag.pointer;
      selection = drag.previousSelection;
      drag = null;
      $("map").releasePointerCapture(pointer);
    }
    rangeSelection = null;
    draw();
    describe();
    return;
  }
  if (
    !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key) ||
    !columns.length
  )
    return;
  e.preventDefault();
  const extend = e.shiftKey && ["ArrowLeft", "ArrowRight"].includes(e.key);
  let i = columns.indexOf(
    extend && rangeSelection ? rangeSelection.focus : selection.id,
  );
  selection.r = Math.max(
    0,
    Math.min(
      rows.length - 1,
      selection.r + (e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0),
    ),
  );
  i = Math.max(
    0,
    Math.min(
      columns.length - 1,
      i + (e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0),
    ),
  );
  if (extend) selectRange(selection.r, selection.id, columns[i]);
  else {
    rangeSelection = null;
    selection.id = columns[i];
  }
  draw();
  describe();
  const x = i * cw;
  if (
    x < $("scroll").scrollLeft ||
    x + cw > $("scroll").scrollLeft + $("scroll").clientWidth
  )
    $("scroll").scrollLeft = Math.max(0, x - $("scroll").clientWidth / 2);
});
$("scroll").addEventListener(
  "wheel",
  (e) => {
    if (!e.ctrlKey) return;
    e.preventDefault();
    const offset = e.clientX - $("scroll").getBoundingClientRect().left;
    const fraction = ($("scroll").scrollLeft + offset) / width;
    $("zoom").value = String(
      Math.max(1, Math.min(12, +$("zoom").value + (e.deltaY < 0 ? 0.5 : -0.5))),
    );
    draw();
    $("scroll").scrollLeft = fraction * width - offset;
  },
  { passive: false },
);
D.runs.forEach((r, i) => {
  const name =
    r.name +
    " · " +
    (r.meta.source_revision || "unknown").slice(0, 10) +
    " · " +
    (r.meta.created_at || "").slice(0, 16);
  option($("run"), i, name);
  option($("compare"), i, name);
});
for (const g of new Set(D.columns.map(group))) option($("group"), g, g);
for (const p of new Set(Object.values(D.definitions).map((c) => c.fixture)))
  option($("profile"), p, p);
$("compare").value = D.runs.length > 1 ? "1" : "0";
refreshReferences();
for (const id of ["mode", "reference", "compare", "group", "profile", "order"])
  $(id).addEventListener("change", render);
$("run").addEventListener("change", () => {
  rangeSelection = null;
  refreshReferences();
  render();
});
$("search").addEventListener("input", render);
$("zoom").addEventListener("input", draw);
$("fit").addEventListener("click", () => {
  rangeSelection = null;
  $("zoom").value = "1";
  $("group").value = "";
  $("profile").value = "";
  $("search").value = "";
  $("order").value = "catalog";
  $("scroll").scrollLeft = 0;
  render();
});
$("clearRange").addEventListener("click", () => {
  rangeSelection = null;
  draw();
});
new ResizeObserver(draw).observe($("scroll"));
render();
