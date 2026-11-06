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

// ---------- rendering ----------
function bandFor(cls, sources, extra) {
  const primary = sources[0];
  let detail;
  if (cls === "verified") detail = primary ? `From ${primary.title}` : "From an applicable official document";
  else if (cls === "partial") detail = "Related material only. Not the exact document for your question.";
  else detail = "Nothing installed answers this reliably.";
  if (extra) detail = extra;
  return el("div", { class: `band ${cls}` }, el("strong", {}, LABEL[cls]), el("span", {}, detail));
}

function renderAnswerText(text, citations, consulted, card) {
  const body = el("div", { class: "body" });
  const byMarker = new Map(citations.map((c) => [c.marker, c]));
  const parts = text.split(/(\[S\d{1,3}\])/g);
  for (const part of parts) {
    const m = part.match(/^\[S(\d{1,3})\]$/);
    if (m && byMarker.has(Number(m[1]))) {
      const n = Number(m[1]);
      const hit = consulted[n - 1];
      body.append(el("button", {
        class: "cite", type: "button", "aria-label": `Open source ${n}: ${hit ? hit.title : ""}`,
        on: { click: () => openReader(hit) },
      }, String(n)));
    } else if (part) {
      body.append(document.createTextNode(part));
    }
  }
  card.append(body);
}

function renderSources(hits, card, heading, withPreview) {
  if (!hits.length) return;
  const list = el("ol");
  hits.forEach((h) => {
    const item = el("li", {},
      el("button", { type: "button", on: { click: () => openReader(h) } }, h.title || h.doc_id),
      h.heading ? el("span", { class: "heading" }, `, ${h.heading}`) : null);
    if (withPreview) item.append(el("p", { class: "passage-preview" }, h.text.slice(0, 260) + (h.text.length > 260 ? "…" : "")));
    list.append(item);
  });
  card.append(el("div", { class: "sources" }, el("h3", {}, heading), list));
}

function renderNotes(notes, card) {
  if (notes && notes.length) card.append(el("ul", { class: "notes" }, notes.map((n) => el("li", {}, n))));
}

function newCard(question) {
  const card = el("article", { class: "answer new" }, el("p", { class: "q" }, question));
  $("results").prepend(card);
  $("empty").hidden = true;
  return card;
}

function renderResponse(card, data) {
  card.querySelector(".band")?.remove();
  if (data.mode === "answer") {
    const a = data.answer;
    const cited = a.citations.map((c) => a.consulted[c.marker - 1]).filter(Boolean);
    card.append(bandFor(a.class, cited));
    renderNotes(a.notes, card);
    if (a.text) renderAnswerText(a.text, a.citations, a.consulted, card);
    renderSources(a.consulted, card, a.text ? "Sources consulted" : "Closest passages in the library", !a.text);
  } else {
    const s = data.assessment;
    const reason = data.status && (data.status.reason || data.status.error);
    card.append(bandFor(s.class, s.citable.filter((h) => s.verified_keys.includes(`${h.package_id}/${h.passage_id}`)),
      reason ? `Showing library passages. ${reason}` : null));
    renderNotes(s.notes, card);
    renderSources(s.citable, card, "Passages from the library", true);
  }
}

async function submit(kind) {
  const q = $("question").value.trim();
  if (!q) { $("question").focus(); return; }
  const card = newCard(q);
  card.append(el("div", { class: "band pending" }, el("strong", {}, kind === "ask" ? "Reading the library…" : "Searching…")));
  for (const b of ["ask-btn", "search-btn"]) $(b).disabled = true;
  try {
    const r = await fetch(kind === "ask" ? "/api/ask" : "/api/search", {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ question: q, ...context() }),
    });
    const data = await r.json();
    if (!r.ok) throw new Error(data.error || `Request failed (${r.status})`);
    renderResponse(card, kind === "ask" ? data : { mode: "search_only", assessment: data });
  } catch (e) {
    card.querySelector(".band")?.remove();
    card.append(el("div", { class: "band unsupported" }, el("strong", {}, "The request did not complete"), el("span", {}, String(e.message || e))));
  } finally {
    for (const b of ["ask-btn", "search-btn"]) $(b).disabled = false;
    refreshStatus();
  }
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
  $("tab-library").addEventListener("click", () => showView("library"));
  $("tab-map").addEventListener("click", () => showView("map").catch((e) => { $("map-note").textContent = String(e.message || e); }));
  $("place-q").addEventListener("input", searchPlaces);
  $("dir-clear").addEventListener("click", clearDirections);
  setupContext();
  refreshStatus();
  applyProfile();
  $("ask-form").addEventListener("submit", (e) => { e.preventDefault(); submit("ask"); });
  $("search-btn").addEventListener("click", () => submit("search"));
  $("question").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submit("ask"); }
  });
  $("reader-close").addEventListener("click", closeReader);
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !$("reader").hidden) closeReader(); });
});
