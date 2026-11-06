// Order lifecycle. Payment (real Stripe webhook or test-mode simulation) issues signed
// profile v1 and queues a drive build; each configuration change issues the next version.
import { EDITIONS, validateConfig } from "./catalog.mjs";
import { buildProfile, signProfile } from "./profile.mjs";
import { newId, read, tx } from "./store.mjs";

export function createOrder(userId, input, delivery = "usb") {
  const { config, error } = validateConfig(input);
  if (error) return { error };
  const order = {
    id: newId("ord"),
    user_id: userId,
    status: "pending_payment",
    created_at: new Date().toISOString(),
    amount_cents: EDITIONS[config.edition].price_cents,
    delivery: delivery === "download" ? "download" : "usb",
    config,
    payment: null,
    profiles: [],
    builds: [],
  };
  tx((db) => db.orders.push(order));
  return { order };
}

function issueProfile(order) {
  const version = order.profiles.length + 1;
  const signed = signProfile(buildProfile(order, version));
  order.profiles.push({ version, issued_at: new Date().toISOString(), ...signed });
  order.builds.push({ id: newId("bld"), profile_version: version, status: "queued", updated_at: new Date().toISOString(), note: "" });
}

/** Idempotent: a repeated webhook does not issue a second profile. */
export function markPaid(orderId, payment) {
  return tx((db) => {
    const o = db.orders.find((x) => x.id === orderId);
    if (!o) return { error: "no such order" };
    if (o.status === "pending_payment") {
      o.status = "paid";
      o.payment = { ...payment, at: new Date().toISOString() };
      issueProfile(o);
    }
    return { order: o };
  });
}

/** Change a paid order's configuration (same edition) and queue an updated build. */
export function updateConfig(orderId, userId, input) {
  return tx((db) => {
    const o = db.orders.find((x) => x.id === orderId && x.user_id === userId);
    if (!o) return { error: "no such order" };
    if (o.status !== "paid") return { error: "Only paid orders can be reconfigured." };
    const { config, error } = validateConfig(input, { edition: o.config.edition });
    if (error) return { error };
    o.config = config;
    issueProfile(o);
    return { order: o };
  });
}

export function ordersFor(userId) {
  return read((db) => db.orders.filter((o) => o.user_id === userId).sort((a, b) => b.created_at.localeCompare(a.created_at)));
}

export function orderFor(orderId, userId) {
  return read((db) => db.orders.find((o) => o.id === orderId && o.user_id === userId) || null);
}

// ---- factory side (admin token) ----

export function queuedBuilds() {
  return read((db) =>
    db.orders.flatMap((o) =>
      o.builds
        .filter((b) => b.status === "queued")
        .map((b) => ({ order_id: o.id, build_id: b.id, delivery: o.delivery, profile: o.profiles.find((p) => p.version === b.profile_version) })),
    ),
  );
}

export function setBuildStatus(buildId, status, note = "") {
  if (!["queued", "building", "ready", "shipped", "failed"].includes(status)) return { error: "bad status" };
  return tx((db) => {
    for (const o of db.orders) {
      const b = o.builds.find((x) => x.id === buildId);
      if (b) {
        Object.assign(b, { status, note: String(note).slice(0, 500), updated_at: new Date().toISOString() });
        return { build: b };
      }
    }
    return { error: "no such build" };
  });
}
