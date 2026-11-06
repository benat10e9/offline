import test from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

process.env.STORE_DIR = fs.mkdtempSync(path.join(os.tmpdir(), "store-"));
process.env.KB_PROFILE_SIGNING_KEY = crypto.randomBytes(32).toString("hex");
const { createOrder, markPaid, updateConfig, queuedBuilds, setBuildStatus, ordersFor } = await import("./orders.mjs");
const { publicKeyHex } = await import("./profile.mjs");

const cfg = { edition: "standard", os: ["macos", "windows"], state: "TX", assets: [{ label: "Home generator", make: "Generac", model: "GP6500" }], grades: ["3", "7"] };

function verifies(p) {
  const [, pub, sig] = p.profile_sig.trim().split(" ");
  const key = crypto.createPublicKey({ key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(pub, "hex")]), format: "der", type: "spki" });
  return crypto.verify(null, Buffer.from(p.profile_json), key, Buffer.from(sig, "hex"));
}

test("order -> pay -> signed profile -> update -> v2", () => {
  const { order } = createOrder("usr_1", cfg);
  assert.equal(order.status, "pending_payment");
  assert.equal(order.amount_cents, 9900);

  const paid = markPaid(order.id, { method: "test" }).order;
  assert.equal(paid.status, "paid");
  assert.equal(paid.profiles.length, 1);
  markPaid(order.id, { method: "test" }); // duplicate webhook
  assert.equal(ordersFor("usr_1")[0].profiles.length, 1, "payment is idempotent");

  const p1 = JSON.parse(paid.profiles[0].profile_json);
  assert.deepEqual(p1.packages, ["medical-survival", "maps-texas"]);
  assert.equal(p1.jurisdiction, "US-TX");
  assert.equal(p1.model, "qwen3-8b-q4km");
  assert.ok(p1.requests.some((r) => r.includes("Generac GP6500")));
  assert.ok(verifies(paid.profiles[0]));
  assert.ok(paid.profiles[0].profile_sig.includes(publicKeyHex()));

  const upd = updateConfig(order.id, "usr_1", { ...cfg, state: "CA", model: "qwen3-4b-q4km", edition: "family" }).order;
  const p2 = JSON.parse(upd.profiles[1].profile_json);
  assert.equal(p2.version, 2);
  assert.equal(p2.edition, "standard", "edition cannot change through reconfiguration");
  assert.deepEqual(p2.packages, ["medical-survival"], "no California map yet");
  assert.ok(p2.requests.some((r) => r.includes("US-CA")));
  assert.ok(verifies(upd.profiles[1]));

  const q = queuedBuilds();
  assert.equal(q.length, 2);
  assert.ok(setBuildStatus(q[0].build_id, "ready").build);
  assert.equal(queuedBuilds().length, 1);
  assert.ok(setBuildStatus(q[1].build_id, "bogus").error);
});

test("validation and ownership", () => {
  assert.ok(createOrder("u", { ...cfg, edition: "platinum" }).error);
  assert.ok(createOrder("u", { ...cfg, os: [] }).error);
  assert.ok(createOrder("u", { ...cfg, state: "XX" }).error);
  const { order } = createOrder("owner", cfg);
  assert.ok(updateConfig(order.id, "owner", cfg).error, "unpaid orders cannot be reconfigured");
  markPaid(order.id, {});
  assert.ok(updateConfig(order.id, "someone-else", cfg).error, "other users cannot reconfigure");
});
