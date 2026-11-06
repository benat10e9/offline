import { queuedBuilds } from "../../../../lib/orders.mjs";
import { adminOk, json } from "../../../../lib/session.mjs";

// Factory: list builds waiting to be made (signed profiles included).
export async function GET(request) {
  if (!adminOk(request)) return json({ error: "unauthorized" }, 401);
  return json(queuedBuilds());
}
