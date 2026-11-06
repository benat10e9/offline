// Password hashing (scrypt) and HMAC-signed session cookies, all from node:crypto.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

function sessionSecret() {
  if (process.env.SESSION_SECRET) return process.env.SESSION_SECRET;
  if (process.env.NODE_ENV === "production" && !process.env.ALLOW_DEV_SECRETS) {
    throw new Error("SESSION_SECRET is required in production");
  }
  const f = path.join(process.env.STORE_DIR || path.join(process.cwd(), "data"), "secret");
  try {
    return fs.readFileSync(f, "utf8");
  } catch {
    fs.mkdirSync(path.dirname(f), { recursive: true });
    const s = crypto.randomBytes(32).toString("hex");
    fs.writeFileSync(f, s, { mode: 0o600 });
    return s;
  }
}

export function hashPassword(password, salt = crypto.randomBytes(16).toString("hex")) {
  return { salt, hash: crypto.scryptSync(password, salt, 64).toString("hex") };
}

export function checkPassword(password, salt, hash) {
  const got = Buffer.from(hashPassword(password, salt).hash, "hex");
  const want = Buffer.from(hash, "hex");
  return got.length === want.length && crypto.timingSafeEqual(got, want);
}

const mac = (v) => crypto.createHmac("sha256", sessionSecret()).update(v).digest("base64url");

export function sessionValue(userId) {
  const v = `${userId}.${Date.now() + 30 * 24 * 3600 * 1000}`;
  return `${v}.${mac(v)}`;
}

export function userFromSession(value) {
  if (!value) return null;
  const parts = String(value).split(".");
  if (parts.length !== 3) return null;
  const [id, exp, sig] = parts;
  const want = mac(`${id}.${exp}`);
  if (sig.length !== want.length || !crypto.timingSafeEqual(Buffer.from(sig), Buffer.from(want))) return null;
  if (Number(exp) < Date.now()) return null;
  return id;
}

export const COOKIE = "olsess";
export const cookieOptions = {
  httpOnly: true,
  sameSite: "lax",
  secure: process.env.NODE_ENV === "production" && !process.env.INSECURE_COOKIES,
  path: "/",
  maxAge: 30 * 24 * 3600,
};
