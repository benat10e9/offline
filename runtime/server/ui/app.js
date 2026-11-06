// All text from the library or the model is inserted with textContent, never as HTML.
"use strict";

const $ = (id) => document.getElementById(id);
const STATES = ["AL","AK","AZ","AR","CA","CO","CT","DE","DC","FL","GA","HI","ID","IL","IN","IA","KS","KY","LA","ME","MD","MA","MI","MN","MS","MO","MT","NE","NV","NH","NJ","NM","NY","NC","ND","OH","OK","OR","PA","RI","SC","SD","TN","TX","UT","VT","VA","WA","WV","WI","WY"];
const LABEL = {
  verified: "Verified",
  partial: "Partially supported",
  unsupported: "Not supported by the library",
};

function el(tag, attrs = {}, ...children) {
  const n = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") n.className = v;
    else if (k === "on") for (const [ev, fn] of Object.entries(v)) n.addEventListener(ev, fn);
    else n.setAttribute(k, v);
  }
  for (const c of children.flat()) if (c != null) n.append(c instanceof Node ? c : document.createTextNode(String(c)));
  return n;
}

function store(key, value) {
  try { if (value === undefined) return localStorage.getItem(key) || ""; localStorage.setItem(key, value); } catch { return ""; }
  return "";
}

// ---------- context (equipment + state) ----------
function setupContext() {
  const sel = $("jurisdiction");
  for (const s of STATES) sel.append(el("option", { value: "US-" + s }, s));
  $("asset-make").value = store("asset-make");
  $("asset-model").value = store("asset-model");
  sel.value = store("jurisdiction");
  for (const id of ["asset-make", "asset-model", "jurisdiction"]) {
    $(id).addEventListener("change", () => { store(id, $(id).value); summarizeContext(); });
  }
  summarizeContext();
}

function slug(s) { return s.trim().toLowerCase().split(/\s+/).filter(Boolean).join("-"); }

function context() {
  const make = slug($("asset-make").value), model = slug($("asset-model").value);
  return {
    asset: make && model ? `${make}:${model}` : null,
    jurisdiction: $("jurisdiction").value || null,
  };
}

function summarizeContext() {
  const make = $("asset-make").value.trim(), model = $("asset-model").value.trim();
  const st = $("jurisdiction").value.replace("US-", "");
  const parts = [];
  if (make && model) parts.push(`${make} ${model}`);
  if (st) parts.push(st);
  $("context-summary").textContent = parts.length ? `Equipment and location: ${parts.join(", ")}` : "Equipment and location: not set";
}

// ---------- status ----------
async function refreshStatus() {
  try {
    const r = await fetch("/api/status");
    const s = await r.json();
    const passages = s.packages.reduce((a, p) => a + p.passages, 0);
    $("tab-map").hidden = !s.maps;
    const lib = $("lib-status");
    lib.textContent = s.packages.length
      ? `Library: ${s.packages.length} package${s.packages.length > 1 ? "s" : ""}, ${passages.toLocaleString()} passages`
      : "Library: nothing installed";
    lib.className = "pill" + (s.packages.length ? " ok" : " warn");
    if (s.problems.length) { lib.textContent += `, ${s.problems.length} failed verification`; lib.className = "pill warn"; }

    const m = s.model, ms = $("model-status");
    const text = {
      ready: () => `Assistant ready (${m.backend})`,
      starting: () => "Assistant starting…",
      search_only: () => "Library search only",
      failed: () => "Assistant failed to start",
    }[m.state];
    ms.textContent = text ? text() : "Assistant busy";
    ms.title = m.reason || m.error || m.model || "";
    ms.className = "pill" + (m.state === "ready" ? " ok" : m.state === "starting" ? "" : " warn");
    $("dev-warning").hidden = !(m.state === "ready" && m.unevaluated);
    if (m.state === "starting") setTimeout(refreshStatus, 2000);
  } catch {
    $("model-status").textContent = "App engine not responding";
    $("model-status").className = "pill warn";
  }
}

// ---------- chat ----------
const LABEL_SHORT = { verified: "Verified source", partial: "Partly supported", unsupported: "Not in the library" };
let currentConv = null;
let sending = false;

function inline(text, citations, consulted) {
  // **bold** and [S#] citation chips; everything else as plain text.
  const frag = document.createDocumentFragment();
  const byMarker = new Map((citations || []).map((c) => [c.marker, c]));
  for (const part of text.split(/(\*\*[^*\n]+\*\*|\[S\d{1,3}\])/g)) {
    if (!part) continue;
    const m = part.match(/^\[S(\d{1,3})\]$/);
    if (m) {
      const n = Number(m[1]);
      if (byMarker.has(n) && consulted && consulted[n - 1]) {
        const hit = consulted[n - 1];
        frag.append(el("button", { class: "cite", type: "button", "aria-label": `Open source ${n}: ${hit.title}`, on: { click: () => openReader(hit) } }, String(n)));
      }
    } else if (/^\*\*[^*]+\*\*$/.test(part)) {
      frag.append(el("strong", {}, part.slice(2, -2)));
    } else {
      frag.append(document.createTextNode(part));
    }
  }
  return frag;
}

/** Minimal Markdown: paragraphs, numbered and bulleted lists, headings, bold, citations. */
function renderMarkdown(text, citations, consulted) {
  const out = el("div", { class: "md" });
  let list = null, listType = null, para = [];
  const flushPara = () => { if (para.length) { out.append(el("p", {}, inline(para.join(" "), citations, consulted))); para = []; } };
  const flushList = () => { if (list) { out.append(list); list = null; listType = null; } };
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    let m;
    if (!line) { flushPara(); flushList(); continue; }
    if ((m = line.match(/^(\d{1,2})[.)]\s+(.*)$/))) {
      flushPara();
      if (listType !== "ol") { flushList(); list = el("ol"); listType = "ol"; }
      list.append(el("li", {}, inline(m[2], citations, consulted)));
    } else if ((m = line.match(/^[-*•]\s+(.*)$/))) {
      flushPara();
      if (listType !== "ul") { flushList(); list = el("ul"); listType = "ul"; }
      list.append(el("li", {}, inline(m[1], citations, consulted)));
    } else if ((m = line.match(/^#{1,4}\s+(.*)$/))) {
      flushPara(); flushList();
      out.append(el("h4", {}, inline(m[1], citations, consulted)));
    } else {
      flushList();
      para.push(line);
    }
  }
  flushPara(); flushList();
  return out;
}

function sourceList(hits, title) {
  const list = el("ol");
  hits.forEach((h) => list.append(el("li", {},
    el("button", { type: "button", on: { click: () => openReader(h) } }, h.title || h.doc_id),
    h.heading ? el("span", { class: "heading" }, `, ${h.heading}`) : null)));
  return el("details", { class: "msg-sources" }, el("summary", {}, title), list);
}

/** Normalised view of a stored message or a final stream event. */
function viewOf(data, content) {
  if (data.mode === "search_only") {
    const a = data.assessment || data;
    return { mode: "search_only", cls: a.class, text: null, citations: [], consulted: a.citable || a.consulted || [], notes: a.notes || [], reason: data.status && (data.status.reason || data.status.error) };
  }
  const a = data.answer || data;
  return { mode: "answer", cls: a.class, text: a.text != null ? a.text : content, citations: a.citations || [], consulted: a.consulted || [], notes: a.notes || [] };
}

function renderAssistant(bubble, v) {
  bubble.replaceChildren();
  bubble.append(el("span", { class: `evidence ${v.cls}` }, LABEL_SHORT[v.cls] || v.cls));
  if (v.mode === "answer" && v.text) {
    bubble.append(renderMarkdown(v.text, v.citations, v.consulted));
  } else if (v.mode === "answer" && !v.consulted.length) {
    bubble.append(el("p", {}, "I couldn't answer that from your offline library."));
  } else if (v.consulted.length) {
    if (v.mode === "answer") v.reason = null;
    bubble.append(el("p", {}, v.reason ? `The assistant isn't running (${v.reason}). Here is what the library has:`
      : v.mode === "answer" ? "I couldn't write a reliable answer, but these passages are the closest match:" : "Here is what the library has:"));
    const ul = el("ul", { class: "passages" });
    v.consulted.slice(0, 5).forEach((h) => ul.append(el("li", {},
      el("button", { type: "button", class: "linklike", on: { click: () => openReader(h) } }, h.title),
      el("p", { class: "passage-preview" }, h.text.slice(0, 240) + (h.text.length > 240 ? "…" : "")))));
    bubble.append(ul);
  } else {
    bubble.append(el("p", {}, "Nothing in your offline library answers this."));
  }
  if (v.notes.length) bubble.append(el("ul", { class: "notes" }, v.notes.map((n) => el("li", {}, n))));
  if (v.mode === "answer" && v.text) {
    const citedIdx = new Set(v.citations.map((c) => c.marker - 1));
    const cited = v.consulted.filter((_, i) => citedIdx.has(i));
    const others = v.consulted.filter((_, i) => !citedIdx.has(i));
    if (cited.length) bubble.append(sourceList(cited, `Sources (${cited.length})`));
    if (others.length) bubble.append(sourceList(others, `Other passages checked (${others.length})`));
  }
}

function addUser(text) {
  const remember = el("button", { type: "button", class: "remember", title: "Save this as a note the assistant will remember",
    on: { click: async (e) => {
      const r = await fetch("/api/memories", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ text }) });
      e.target.textContent = r.ok ? "Remembered" : "Couldn't save";
      e.target.disabled = true;
      memoriesRendered = false;
    } } }, "Remember");
  const row = el("div", { class: "msg user" }, el("div", { class: "bubble" }, text), remember);
  $("thread").append(row);
  return row;
}

function addAssistant() {
  const bubble = el("div", { class: "bubble" });
  $("thread").append(el("div", { class: "msg assistant" }, bubble));
  return bubble;
}

function scrollThread() { const t = $("thread"); t.scrollTop = t.scrollHeight; }

function showWelcome() {
  $("thread").replaceChildren(el("div", { class: "welcome" },
    el("h2", {}, "What do you need to know?"),
    el("p", {}, "Ask in plain words. Answers come from your offline library and show where they came from. Follow-up questions keep the context."),
    el("div", { class: "starters" }, ["How do I purify river water?", "How do I treat a burn?", "Can I run a generator in my garage?", "What are signs of heat stroke?"]
      .map((q) => el("button", { type: "button", on: { click: () => { $("question").value = q; send(); } } }, q)))));
}

async function loadConversations() {
  try {
    const list = await (await fetch("/api/conversations")).json();
    const ol = $("chat-list");
    ol.replaceChildren(...list.map((c) => el("li", { class: c.id === currentConv ? "active" : "" },
      el("button", { type: "button", class: "conv", on: { click: () => openConversation(c.id) } }, c.title || "New chat"),
      el("button", { type: "button", class: "del", "aria-label": `Delete ${c.title}`, title: "Delete",
        on: { click: async () => { await fetch(`/api/conversations/${c.id}`, { method: "DELETE" }); if (c.id === currentConv) newChat(); loadConversations(); } } }, "×"))));
  } catch { /* storage unavailable: chat still works per message */ }
}

async function openConversation(id) {
  currentConv = id;
  const msgs = await (await fetch(`/api/conversations/${id}`)).json();
  $("thread").replaceChildren();
  for (const m of msgs) {
    if (m.role === "user") addUser(m.content);
    else renderAssistant(addAssistant(), viewOf(m.meta || { mode: "answer", class: "unsupported" }, m.content));
  }
  scrollThread();
  loadConversations();
}

function newChat() {
  currentConv = null;
  showWelcome();
  loadConversations();
  $("question").focus();
}

async function send() {
  const q = $("question").value.trim();
  if (!q || sending) return;
  sending = true;
  $("ask-btn").disabled = true;
  $("question").value = "";
  autosize();
  if (!$("thread").querySelector(".msg")) $("thread").replaceChildren();
  try {
    if (currentConv == null) {
      const r = await fetch("/api/conversations", { method: "POST" });
      currentConv = (await r.json()).id;
    }
    addUser(q);
    const bubble = addAssistant();
    bubble.append(el("span", { class: "evidence pending" }, "Reading the library…"));
    const live = el("div", { class: "md streaming" });
    bubble.append(live);
    scrollThread();
    const res = await fetch(`/api/conversations/${currentConv}/ask`, {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ question: q, ...context() }),
    });
    if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error || `Request failed (${res.status})`);
    const reader = res.body.getReader();
    const dec = new TextDecoder();
    let buf = "", streamed = "";
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buf += dec.decode(value, { stream: true });
      let nl;
      while ((nl = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, nl); buf = buf.slice(nl + 1);
        if (!line.trim()) continue;
        const ev = JSON.parse(line);
        if (ev.type === "token") {
          streamed += ev.text;
          bubble.firstChild.textContent = "Writing…";
          live.textContent = streamed.replace(/<think>[\s\S]*?(<\/think>|$)/g, "").replace(/\[S\d{1,3}\]/g, "");
          scrollThread();
        } else if (ev.type === "final") {
          renderAssistant(bubble, viewOf(ev));
        } else if (ev.type === "error") {
          bubble.replaceChildren(el("span", { class: "evidence unsupported" }, "Error"), el("p", {}, ev.message));
        }
      }
    }
  } catch (e) {
    const b = addAssistant();
    b.append(el("span", { class: "evidence unsupported" }, "Error"), el("p", {}, String(e.message || e)));
  } finally {
    sending = false;
    $("ask-btn").disabled = false;
    scrollThread();
    loadConversations();
    refreshStatus();
  }
}

function autosize() {
  const t = $("question");
  t.style.height = "auto";
  t.style.height = Math.min(t.scrollHeight, 200) + "px";
}

// ---------- memories ----------
let memoriesRendered = false;
async function renderMemories() {
  if (memoriesRendered) return;
  const list = await (await fetch("/api/memories")).json().catch(() => []);
  $("memory-list").replaceChildren(...(list.length ? list.map((m) => el("li", {},
    el("span", {}, m.text),
    el("button", { type: "button", "aria-label": "Delete note", on: { click: async () => {
      await fetch(`/api/memories/${m.id}`, { method: "DELETE" }); memoriesRendered = false; renderMemories(); } } }, "Delete")))
    : [el("li", { class: "hint" }, "No notes yet. Tap Remember on any of your messages, or add one here.")]));
  memoriesRendered = true;
}

// ---------- reader ----------
async function openReader(hit) {
  if (!hit) return;
  const r = await fetch(`/api/document?package=${encodeURIComponent(hit.package_id)}&doc=${encodeURIComponent(hit.doc_id)}`);
  const data = await r.json();
  $("reader-title").textContent = hit.title || hit.doc_id;
  const s = data.source;
  $("reader-meta").textContent = s
    ? [s.publisher, s.revision, `License: ${s.license}`].filter(Boolean).join(". ")
    : "";
  const body = $("reader-body");
  body.replaceChildren();
  let target = null, lastHeading = "";
  for (const p of data.passages || []) {
    if (p.heading && p.heading !== lastHeading) { body.append(el("h4", {}, p.heading)); lastHeading = p.heading; }
    const para = el("p", { class: p.passage_id === hit.passage_id ? "cited" : "" }, p.text);
    if (p.passage_id === hit.passage_id) target = para;
    body.append(para);
  }
  $("reader").hidden = false;
  document.querySelector(".shell").classList.add("reading");
  if (target) target.scrollIntoView({ block: "center" });
  $("reader-close").focus();
}

function closeReader() {
  $("reader").hidden = true;
  document.querySelector(".shell").classList.remove("reading");
}

// ---------- library ----------
let libraryRendered = false;
async function renderLibrary() {
  if (libraryRendered) return;
  const pkgs = await (await fetch("/api/library")).json();
  const list = $("library-list");
  for (const p of pkgs) {
    const rows = p.sources.map((s) => el("tr", {},
      el("td", {}, s.title, s.audience === "clinician" ? el("div", { class: "hint" }, "Written for health professionals") : null),
      el("td", {}, s.publisher),
      el("td", {}, s.revision || ""),
      el("td", {}, s.authority),
      el("td", {}, s.license)));
    list.append(el("section", { class: "lib-pkg" },
      el("h3", {}, `${p.title} (${p.version})`),
      rows.length ? el("div", { class: "lib-wrap" }, el("table", { class: "lib-table" },
        el("thead", {}, el("tr", {}, ["Source", "Publisher", "Revision", "Authority", "License"].map((h) => el("th", {}, h)))),
        el("tbody", {}, rows))) : el("p", { class: "hint" }, "Map data")));
  }
  libraryRendered = true;
}

// ---------- profile ----------
async function applyProfile() {
  const p = await (await fetch("/api/profile")).json();
  if (!p) return;
  if (p.jurisdiction && !store("jurisdiction")) { $("jurisdiction").value = p.jurisdiction; summarizeContext(); }
  if (p.assets && p.assets.length) {
    const sel = $("my-equipment");
    p.assets.forEach((a, i) => sel.append(el("option", { value: String(i) }, `${a.label}: ${a.make} ${a.model}`)));
    $("my-equipment-label").hidden = false;
    sel.addEventListener("change", () => {
      const a = p.assets[Number(sel.value)];
      $("asset-make").value = a ? a.make : "";
      $("asset-model").value = a ? a.model : "";
      for (const id of ["asset-make", "asset-model"]) store(id, $(id).value);
      summarizeContext();
    });
  }
}

// ---------- map ----------
let mapState = null;

function loadScript(src) {
  return new Promise((ok, fail) => {
    const s = el("script", { src });
    s.addEventListener("load", ok);
    s.addEventListener("error", () => fail(new Error(`Could not load ${src}`)));
    document.head.append(s);
  });
}

async function showView(view) {
  const map = view === "map";
  $("mapview").hidden = !map;
  $("askview").hidden = view !== "ask";
  $("libraryview").hidden = view !== "library";
  for (const v of ["ask", "map", "library"]) $(`tab-${v}`).setAttribute("aria-pressed", String(view === v));
  if (view === "library") await renderLibrary();
  if (map) {
    if (!mapState) await initMap();
    else mapState.map.resize();
    $("place-q").focus();
  }
}

async function initMap() {
  mapState = { map: null, marker: null };
  const maps = await (await fetch("/api/maps")).json();
  if (!maps.length) {
    $("map-note").textContent = "No map is installed on this drive. Map packages add offline maps and place search.";
    return;
  }
  const m = maps[0];
  document.head.append(el("link", { rel: "stylesheet", href: "/vendor/maplibre-gl.css" }));
  await loadScript("/vendor/maplibre-gl-csp.js");
  await loadScript("/vendor/pmtiles.js");
  await loadScript("/vendor/basemaps.js");
  /* global maplibregl, pmtiles, basemaps */
  maplibregl.setWorkerUrl("/vendor/maplibre-gl-csp-worker.js");
  maplibregl.addProtocol("pmtiles", new pmtiles.Protocol().tile);
  const base = `${location.origin}/maps/${encodeURIComponent(m.package_id)}`;
  const dark = matchMedia("(prefers-color-scheme: dark)").matches;
  const flavor = dark ? "dark" : "light";
  const [w, s, e, n] = m.meta.bounds;
  const map = new maplibregl.Map({
    container: "map",
    style: {
      version: 8,
      glyphs: `${base}/assets/fonts/{fontstack}/{range}.pbf`,
      sprite: `${base}/assets/sprites/v4/${flavor}`,
      sources: {
        protomaps: { type: "vector", url: `pmtiles://${base}/${m.meta.tiles}`, attribution: m.meta.attribution },
      },
      layers: basemaps.layers("protomaps", basemaps.namedFlavor(flavor), { lang: "en" }),
    },
    bounds: [[w, s], [e, n]],
    maxBounds: [[w - 2, s - 2], [e + 2, n + 2]],
    maxZoom: (m.meta.maxzoom || 14) + 2,
  });
  map.addControl(new maplibregl.NavigationControl({ visualizePitch: false }), "top-right");
  map.addControl(new maplibregl.ScaleControl({ unit: "imperial" }), "bottom-left");
  mapState.map = map;
  mapState.routing = maps.some((x) => x.routing);
  $("map-note").textContent = `${m.meta.title || m.meta.region}: ${Number(m.meta.places || 0).toLocaleString()} places searchable offline.`;
}

let placeTimer = null;
function searchPlaces() {
  clearTimeout(placeTimer);
  placeTimer = setTimeout(async () => {
    const q = $("place-q").value.trim();
    const list = $("place-results");
    if (!q) { list.replaceChildren(); return; }
    const places = await (await fetch(`/api/places?q=${encodeURIComponent(q)}&limit=12`)).json();
    list.replaceChildren(...places.map((p) => el("li", {},
      el("button", { type: "button", on: { click: () => goToPlace(p) } },
        el("span", {}, p.name),
        el("span", { class: "pk" }, [p.kind, p.context].filter(Boolean).join(", "))),
      mapState && mapState.routing ? el("button", { type: "button", class: "route-btn", "aria-label": `Directions from ${p.name}`, on: { click: () => setEnd("from", p) } }, "From") : null,
      mapState && mapState.routing ? el("button", { type: "button", class: "route-btn", "aria-label": `Directions to ${p.name}`, on: { click: () => setEnd("to", p) } }, "To") : null)));
    if (!places.length) list.append(el("li", { class: "hint" }, "No matching places in the offline map."));
  }, 150);
}

// ---------- directions ----------
const ends = { from: null, to: null };

function setEnd(which, p) {
  ends[which] = p;
  $("directions").hidden = false;
  $(`dir-${which}`).textContent = p.name;
  if (ends.from && ends.to) routeNow();
  else goToPlace(p);
}

function fmtDuration(s) {
  const m = Math.round(s / 60);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
}

async function routeNow() {
  const sum = $("dir-summary");
  sum.textContent = "Finding a route…";
  const q = `from=${ends.from.lat},${ends.from.lon}&to=${ends.to.lat},${ends.to.lon}`;
  const r = await fetch(`/api/route?${q}`);
  const data = await r.json();
  if (!r.ok) { sum.textContent = data.error || "No route found."; clearRouteLine(); return; }
  const route = data.route;
  const miles = route.distance_m / 1609.344;
  sum.replaceChildren(
    document.createTextNode(`${miles.toFixed(miles < 10 ? 1 : 0)} mi, about ${fmtDuration(route.duration_s)} by car`),
    el("small", {}, "Estimated from road types. Turn restrictions and closures are not known offline."));
  drawRoute(route.coordinates);
}

function drawRoute(coords) {
  const map = mapState.map;
  const data = { type: "Feature", geometry: { type: "LineString", coordinates: coords }, properties: {} };
  if (map.getSource("route")) map.getSource("route").setData(data);
  else {
    map.addSource("route", { type: "geojson", data });
    map.addLayer({ id: "route-casing", type: "line", source: "route", layout: { "line-join": "round", "line-cap": "round" }, paint: { "line-color": "#ffffff", "line-width": 9 } });
    map.addLayer({ id: "route", type: "line", source: "route", layout: { "line-join": "round", "line-cap": "round" }, paint: { "line-color": "#1f4e79", "line-width": 5 } });
  }
  const lons = coords.map((c) => c[0]), lats = coords.map((c) => c[1]);
  map.fitBounds([[Math.min(...lons), Math.min(...lats)], [Math.max(...lons), Math.max(...lats)]], { padding: 60, maxZoom: 15 });
}

function clearRouteLine() {
  const map = mapState && mapState.map;
  if (map && map.getSource("route")) map.getSource("route").setData({ type: "FeatureCollection", features: [] });
}

function clearDirections() {
  ends.from = ends.to = null;
  $("dir-from").textContent = $("dir-to").textContent = "Choose a place";
  $("dir-summary").replaceChildren();
  $("directions").hidden = true;
  clearRouteLine();
}

function goToPlace(p) {
  if (!mapState || !mapState.map) return;
  const zoom = ["city", "town"].includes(p.kind) ? 11 : p.kind === "village" || p.kind === "hamlet" ? 13 : 15;
  mapState.map.flyTo({ center: [p.lon, p.lat], zoom, essential: true });
  if (mapState.marker) mapState.marker.remove();
  mapState.marker = new maplibregl.Marker({ color: "#2a7a4b" }).setLngLat([p.lon, p.lat]).addTo(mapState.map);
}

document.addEventListener("DOMContentLoaded", () => {
  $("tab-ask").addEventListener("click", () => showView("ask"));
  $("tab-library").addEventListener("click", () => showView("library").then(renderMemories));
  $("tab-map").addEventListener("click", () => showView("map").catch((e) => { $("map-note").textContent = String(e.message || e); }));
  $("place-q").addEventListener("input", searchPlaces);
  $("dir-clear").addEventListener("click", clearDirections);
  $("new-chat").addEventListener("click", newChat);
  $("memory-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const text = $("memory-text").value.trim();
    if (!text) return;
    await fetch("/api/memories", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ text }) });
    $("memory-text").value = "";
    memoriesRendered = false;
    renderMemories();
  });
  setupContext();
  refreshStatus();
  applyProfile();
  showWelcome();
  loadConversations();
  $("ask-form").addEventListener("submit", (e) => { e.preventDefault(); send(); });
  $("question").addEventListener("input", autosize);
  $("question").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); }
  });
  $("reader-close").addEventListener("click", closeReader);
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !$("reader").hidden) closeReader(); });
});
