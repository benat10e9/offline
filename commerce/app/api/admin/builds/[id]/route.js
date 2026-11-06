import { setBuildStatus } from "../../../../../lib/orders.mjs";
import { adminOk, json } from "../../../../../lib/session.mjs";

// Factory: report build progress (building, ready, shipped, failed).
export async function POST(request, { params }) {
  if (!adminOk(request)) return json({ error: "unauthorized" }, 401);
  const { id } = await params;
  const { status, note } = await request.json().catch(() => ({}));
  const r = setBuildStatus(id, status, note);
  return r.error ? json(r, 400) : json(r);
}
