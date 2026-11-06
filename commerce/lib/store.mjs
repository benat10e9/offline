// A small JSON-file store. One process, synchronous writes with atomic rename: enough for
// test-mode operation. Replace with Postgres before real order volume.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";

const dir = () => process.env.STORE_DIR || path.join(process.cwd(), "data");
const file = () => path.join(dir(), "db.json");

function load() {
  try {
    return JSON.parse(fs.readFileSync(file(), "utf8"));
  } catch {
    return { users: [], orders: [] };
  }
}

function save(db) {
  fs.mkdirSync(dir(), { recursive: true });
  const tmp = `${file()}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(db, null, 1));
  fs.renameSync(tmp, file());
}

/** Read-modify-write the whole store; returns what `fn` returns. */
export function tx(fn) {
  const db = load();
  const out = fn(db);
  save(db);
  return out;
}

export function read(fn) {
  return fn(load());
}

export const newId = (prefix) => `${prefix}_${crypto.randomBytes(9).toString("base64url")}`;
