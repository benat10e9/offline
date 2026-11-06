import { updateConfig } from "../../../../lib/orders.mjs";
import { currentUser, json } from "../../../../lib/session.mjs";

export async function PUT(request, { params }) {
  const user = await currentUser();
  if (!user) return json({ error: "Sign in first." }, 401);
  const { id } = await params;
  const body = await request.json().catch(() => ({}));
  const { order, error } = updateConfig(id, user.id, body.config);
  if (error) return json({ error }, 400);
  return json({ order_id: order.id, profile_version: order.profiles.length });
}
