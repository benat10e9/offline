// End-to-end customer journey against a running store (default http://localhost:3000):
// sign up -> configure -> simulated payment -> signed profile v1 -> change configuration ->
// signed profile v2 -> factory queue sees both builds.
// Writes the profiles to OUT_DIR/v1 and OUT_DIR/v2 for scripts/build-drive.sh.
//   STORE_URL=http://localhost:3000 ADMIN_TOKEN=... OUT_DIR=out CONFIG='{"os":["linux"]}' node scripts/e2e.mjs
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

const BASE = process.env.STORE_URL || "http://localhost:3000";
const OUT = process.env.OUT_DIR || "e2e-out";
let cookie = "";

async function call(method, url, body, headers = {}) {
  const r = await fetch(BASE + url, {
    method,
    headers: { "content-type": "application/json", cookie, ...headers },
    body: body ? JSON.stringify(body) : undefined,
    redirect: "manual",
  });
  const set = r.headers.get("set-cookie");
  if (set) cookie = set.split(";")[0];
  const text = await r.text();
  let data = text;
  try { data = JSON.parse(text); } catch {}
  return { status: r.status, data };
}

const step = (s) => console.log(`\n== ${s}`);
const email = `e2e-${Date.now()}@example.com`;
const override = JSON.parse(process.env.CONFIG || "{}");
const config = { edition: "standard", os: ["macos", "windows"], state: "TX", model: "qwen3-4b-q4km",
  assets: [{ label: "Home generator", make: "Generac", model: "GP6500", year: 2022 }], grades: ["3"], ...override };

step("storefront is up");
const home = await fetch(BASE + "/");
assert.equal(home.status, 200);
assert.match(await home.text(), /Essential/);

step("sign up");
let r = await call("POST", "/api/auth/signup", { email, password: "correct horse battery" });
assert.equal(r.status, 200, JSON.stringify(r.data));
assert.ok(cookie, "session cookie set");

step("configure and start checkout");
r = await call("POST", "/api/orders", { config, delivery: "usb" });
assert.equal(r.status, 200, JSON.stringify(r.data));
const orderId = r.data.order_id;
console.log("order", orderId, "->", r.data.checkout_url);

step("pay (simulated test mode)");
r = await call("POST", "/api/checkout/mock", { order_id: orderId });
assert.equal(r.status, 200, JSON.stringify(r.data));

async function saveProfile(v) {
  const dir = path.join(OUT, `v${v}`);
  fs.mkdirSync(dir, { recursive: true });
  for (const file of ["json", "sig"]) {
    const res = await fetch(`${BASE}/api/orders/${orderId}/profile?v=${v}&file=${file}`, { headers: { cookie } });
    assert.equal(res.status, 200);
    fs.writeFileSync(path.join(dir, `profile.${file}`), await res.text());
  }
  const p = JSON.parse(fs.readFileSync(path.join(dir, "profile.json"), "utf8"));
  console.log(`profile v${v}:`, JSON.stringify({ packages: p.packages, model: p.model, os: p.os, jurisdiction: p.jurisdiction }));
  return p;
}

step("download signed profile v1");
const p1 = await saveProfile(1);
assert.equal(p1.version, 1);

step("portal shows the order");
const portal = await fetch(BASE + "/portal", { headers: { cookie } });
assert.match(await portal.text(), new RegExp(orderId));

step("change configuration (customer moved states)");
const update = { ...config, state: process.env.UPDATE_STATE || "CA" };
r = await call("PUT", `/api/orders/${orderId}`, { config: update });
assert.equal(r.status, 200, JSON.stringify(r.data));
const p2 = await saveProfile(2);
assert.equal(p2.version, 2);

step("factory queue");
const admin = { authorization: `Bearer ${process.env.ADMIN_TOKEN}` };
r = await call("GET", "/api/admin/queue", null, admin);
assert.equal(r.status, 200, JSON.stringify(r.data));
const mine = r.data.filter((b) => b.order_id === orderId);
assert.equal(mine.length, 2, "both profile versions queued");
r = await call("POST", `/api/admin/builds/${mine[0].build_id}`, { status: "ready", note: "e2e" }, admin);
assert.equal(r.status, 200);

step("security checks");
const other = await fetch(`${BASE}/api/orders/${orderId}/profile?v=1&file=json`);
assert.equal(other.status, 401, "profiles need a session");
r = await call("GET", "/api/admin/queue", null, { authorization: "Bearer wrong" });
assert.equal(r.status, 401, "admin needs the token");

console.log(`\nE2E OK: order ${orderId}, profiles written to ${OUT}/v1 and ${OUT}/v2`);
fs.writeFileSync(path.join(OUT, "order.txt"), orderId);
