"use client";
import { useState } from "react";

const blankAsset = { label: "", make: "", model: "", year: "" };

export default function ConfigureForm({ editions, models, states, mapRegions, initial, orderId }) {
  const [cfg, setCfg] = useState(initial);
  const [msg, setMsg] = useState("");
  const [busy, setBusy] = useState(false);
  const ed = editions[cfg.edition];
  const set = (k, v) => setCfg((c) => ({ ...c, [k]: v }));
  const toggleOs = (o) => set("os", cfg.os.includes(o) ? cfg.os.filter((x) => x !== o) : [...cfg.os, o]);
  const setAsset = (i, k, v) => set("assets", cfg.assets.map((a, j) => (j === i ? { ...a, [k]: v } : a)));
  const mapPkg = mapRegions[`US-${cfg.state}`];

  async function submit(e) {
    e.preventDefault();
    setBusy(true);
    setMsg("");
    const r = await fetch(orderId ? `/api/orders/${orderId}` : "/api/orders", {
      method: orderId ? "PUT" : "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ config: cfg, delivery: cfg.delivery }),
    });
    const d = await r.json().catch(() => ({}));
    setBusy(false);
    if (r.status === 401) return (location.href = `/account?next=${encodeURIComponent(location.pathname + location.search)}`);
    if (!r.ok) return setMsg(d.error || "Something went wrong.");
    location.href = orderId ? `/portal?updated=${orderId}` : d.checkout_url;
  }

  return (
    <form onSubmit={submit}>
      <fieldset>
        <legend>Edition</legend>
        {orderId ? <p>{ed.name}</p> : (
          <select value={cfg.edition} onChange={(e) => set("edition", e.target.value)} aria-label="Edition">
            {Object.entries(editions).map(([id, x]) => <option key={id} value={id}>{x.name}</option>)}
          </select>
        )}
      </fieldset>
      <fieldset>
        <legend>Your computers</legend>
        <div className="row">
          {[["macos", "Mac (Apple Silicon)"], ["windows", "Windows PC"]].map(([o, label]) => (
            <label key={o} style={{ display: "flex", gap: 6, alignItems: "center" }}>
              <input type="checkbox" checked={cfg.os.includes(o)} onChange={() => toggleOs(o)} /> {label}
            </label>
          ))}
        </div>
        <label style={{ marginTop: 10 }}>AI model
          <select value={cfg.model} onChange={(e) => set("model", e.target.value)}>
            {Object.entries(models).map(([id, m]) => <option key={id} value={id}>{m.name}: {m.note}</option>)}
          </select>
        </label>
      </fieldset>
      <fieldset>
        <legend>Where you live</legend>
        <label>State
          <select value={cfg.state} onChange={(e) => set("state", e.target.value)}>
            {states.map((s) => <option key={s} value={s}>{s}</option>)}
          </select>
        </label>
        {ed.maps && <p className="muted">{mapPkg ? "Offline map, place search and driving directions for your state are included." : "Your state's offline map isn't available yet. We'll record the request and add it in an update."}</p>}
      </fieldset>
      <fieldset>
        <legend>Your equipment (optional)</legend>
        <p className="muted">Vehicles, generators, pumps. The assistant uses this to tell you when it has your exact manual and when it only has general guidance. Manuals for specific equipment are added as they become available.</p>
        {cfg.assets.map((a, i) => (
          <div className="grid2" key={i} style={{ marginBottom: 8 }}>
            <input placeholder="What is it (e.g. Home generator)" value={a.label} onChange={(e) => setAsset(i, "label", e.target.value)} aria-label="Equipment label" />
            <input placeholder="Make" value={a.make} onChange={(e) => setAsset(i, "make", e.target.value)} aria-label="Make" />
            <input placeholder="Model" value={a.model} onChange={(e) => setAsset(i, "model", e.target.value)} aria-label="Model" />
            <input placeholder="Year" value={a.year || ""} onChange={(e) => setAsset(i, "year", e.target.value)} aria-label="Year" inputMode="numeric" />
          </div>
        ))}
        <button className="btn secondary" type="button" onClick={() => set("assets", [...cfg.assets, { ...blankAsset }])}>Add equipment</button>
      </fieldset>
      <fieldset>
        <legend>School grades at home (optional)</legend>
        <p className="muted">Recorded for upcoming education packages.</p>
        <div className="row">
          {["K", ...Array.from({ length: 12 }, (_, i) => String(i + 1))].map((g) => (
            <label key={g} style={{ display: "flex", gap: 4, alignItems: "center" }}>
              <input type="checkbox" checked={cfg.grades.includes(g)} onChange={() => set("grades", cfg.grades.includes(g) ? cfg.grades.filter((x) => x !== g) : [...cfg.grades, g])} /> {g}
            </label>
          ))}
        </div>
      </fieldset>
      {!orderId && (
        <fieldset>
          <legend>Delivery</legend>
          <select value={cfg.delivery} onChange={(e) => set("delivery", e.target.value)} aria-label="Delivery">
            <option value="usb">USB drive, shipped</option>
            <option value="download">Download, I'll use my own drive</option>
          </select>
        </fieldset>
      )}
      <button className="btn" type="submit" disabled={busy}>{orderId ? "Save and request an updated build" : "Continue to payment"}</button>
      {msg && <p className="err" role="alert">{msg}</p>}
    </form>
  );
}
