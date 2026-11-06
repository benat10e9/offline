// Drive profiles: the signed link between an order and the drive the factory builds.
// Format and signature match the Rust `kb` tool (spec/profile.md): the exact bytes of
// profile.json are signed with Ed25519; profile.sig is "ed25519 <public hex> <signature hex>".
import crypto from "node:crypto";
import { resolvePackages } from "./catalog.mjs";

const PKCS8_ED25519_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");

function signingKey() {
  const hex = (process.env.KB_PROFILE_SIGNING_KEY || "").trim();
  if (!/^[0-9a-f]{64}$/i.test(hex)) throw new Error("KB_PROFILE_SIGNING_KEY must be a 32-byte hex secret (kb keygen format)");
  return crypto.createPrivateKey({ key: Buffer.concat([PKCS8_ED25519_PREFIX, Buffer.from(hex, "hex")]), format: "der", type: "pkcs8" });
}

export function publicKeyHex(key = signingKey()) {
  const spki = crypto.createPublicKey(key).export({ format: "der", type: "spki" });
  return spki.subarray(spki.length - 32).toString("hex");
}

export function buildProfile(order, version) {
  const { packages, requests } = resolvePackages(order.config);
  return {
    format: 1,
    profile_id: order.id,
    version,
    order_id: order.id,
    edition: order.config.edition,
    os: order.config.os,
    model: order.config.model,
    packages,
    jurisdiction: order.config.state,
    assets: order.config.assets.map((a) => ({ label: a.label || `${a.make} ${a.model}`, make: a.make, model: a.model, year: a.year })),
    requests,
  };
}

export function signProfile(profile) {
  const key = signingKey();
  const json = JSON.stringify(profile, null, 2);
  const sig = crypto.sign(null, Buffer.from(json), key).toString("hex");
  return { profile_json: json, profile_sig: `ed25519 ${publicKeyHex(key)} ${sig}\n` };
}
