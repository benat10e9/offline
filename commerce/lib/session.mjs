import { cookies } from "next/headers";
import { COOKIE, userFromSession } from "./auth.mjs";
import { read } from "./store.mjs";

export async function currentUser() {
  const id = userFromSession((await cookies()).get(COOKIE)?.value);
  if (!id) return null;
  return read((db) => db.users.find((u) => u.id === id) || null);
}

export const json = (body, status = 200) => Response.json(body, { status });

export function paymentsMode() {
  if (process.env.STRIPE_SECRET_KEY) return "stripe";
  if (process.env.NODE_ENV !== "production" || process.env.ALLOW_SIMULATED_PAYMENTS === "1") return "simulated";
  return "disabled";
}

export function adminOk(request) {
  const want = process.env.ADMIN_TOKEN;
  const got = (request.headers.get("authorization") || "").replace(/^Bearer /, "");
  return Boolean(want) && got.length === want.length && got === want;
}
