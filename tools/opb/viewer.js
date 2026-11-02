"use strict";
const D = JSON.parse(document.getElementById("opb-data").textContent),
  $ = (id) => document.getElementById(id);
const TOP = 36,
  ROW = 38;
let rows = [],
  columns = [],
  selection = { r: 0, id: D.columns[0] },
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
function ratio(n) {
  return n == null
    ? "—"
    : n.toLocaleString(undefined, { maximumSignificantDigits: 3 }) + "×";
}
function run() {
  return D.runs[+$("run").value];
}
function cellAt(row, id) {
  return row?.cells?.[id] || null;
}
function metric(row, id) {
  const c = cellAt(row, id),
    mode = $("mode").value;
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
    $("mode").value === "absolute" ? "Process wall time" : "Ratio",
    $("mode").value === "absolute"
      ? c.measurement.wall_seconds?.toFixed(3) + " s"
      : ratio(m.value),
  );
  if (c.timing) {
    factsItem("Sample CV", (100 * c.timing.cv).toFixed(1) + "%");
    factsItem(
      (100 * c.timing.confidence).toFixed(0) + "% mean CI",
      time(c.timing.lower) + " – " + time(c.timing.upper),
    );
  }
  let message =
    m.reason ||
    ($("mode").value === "absolute"
      ? "Operation mean; process wall time also includes setup, warmup and analysis."
      : "Ratio for this case; <1× is faster, >1× is slower than the reference.");
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
    c.timing.samples.forEach((n, i) => {
      const bar = document.createElement("span");
      bar.style.height = Math.max(2, (58 * n) / max) + "px";
      bar.title = "Sample " + (i + 1) + ": " + time(n);
      $("samples").append(bar);
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
  let t;
  if ($("mode").value === "absolute") {
    t = (Math.log10(value) - min) / Math.max(0.01, max - min);
    return (
      "rgb(" +
      mix([42, 93, 126], [237, 159, 70], Math.max(0, Math.min(1, t))).join(
        ",",
      ) +
      ")"
    );
  }
  t = Math.max(-6, Math.min(6, Math.log2(value))) / 6;
  return (
    "rgb(" +
    (t < 0
      ? mix([83, 97, 119], [47, 153, 203], -t)
      : mix([83, 97, 119], [237, 91, 65], t)
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
  document.querySelector(".scale").style.background =
    $("mode").value === "absolute"
      ? "linear-gradient(90deg,rgb(42,93,126),rgb(237,159,70))"
      : "linear-gradient(90deg,rgb(47,153,203),rgb(83,97,119),rgb(237,91,65))";
  $("scaleText").textContent =
    $("mode").value === "absolute"
      ? values.length
        ? time(10 ** min)
        : "No numeric cells"
      : "≤1/64×";
  $("scaleEnd").textContent =
    $("mode").value === "absolute"
      ? values.length
        ? time(10 ** max) + " · log scale"
        : ""
      : "≥64× · log₂ scale · 1× neutral";
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
      ctx.fillText(group(columns[start]), start * cw + 4, 22);
      ctx.restore();
    }
    start = end;
  }
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
            ? "#ff5e85"
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
      if (r === selection.r && id === selection.id) {
        ctx.strokeStyle = "#fff";
        ctx.lineWidth = 2;
        ctx.strokeRect(x + 0.5, y + 1, Math.max(1, cw - 1), ROW - 3);
      }
    }),
  );
  $("zoomText").textContent =
    (+$("zoom").value).toFixed(2).replace(/\.00$/, "") + "×";
}
function render() {
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
    ". Scroll to pan when zoomed; Ctrl + wheel to zoom. Arrow keys move the selected cell. Group headers focus a group.";
  $("backendLabel").hidden = $("mode").value !== "backend";
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
    ($("mode").value === "absolute" ? "absolute" : ratio(m.value)) +
    (m.reason ? " · " + m.reason : "");
});
$("map").addEventListener("click", (e) => {
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
  selection = { r: p.r, id: columns[p.i] };
  draw();
  describe();
});
$("map").addEventListener("keydown", (e) => {
  if (
    !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key) ||
    !columns.length
  )
    return;
  e.preventDefault();
  let i = columns.indexOf(selection.id);
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
  selection.id = columns[i];
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
    const fraction = ($("scroll").scrollLeft + e.offsetX) / width;
    $("zoom").value = String(
      Math.max(1, Math.min(12, +$("zoom").value + (e.deltaY < 0 ? 0.5 : -0.5))),
    );
    draw();
    $("scroll").scrollLeft = fraction * width - e.offsetX;
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
  refreshReferences();
  render();
});
$("search").addEventListener("input", render);
$("zoom").addEventListener("input", draw);
$("fit").addEventListener("click", () => {
  $("zoom").value = "1";
  $("group").value = "";
  $("profile").value = "";
  $("search").value = "";
  $("order").value = "catalog";
  $("scroll").scrollLeft = 0;
  render();
});
new ResizeObserver(draw).observe($("scroll"));
render();
