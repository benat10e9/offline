import { markPaid, orderFor } from "../../../../lib/orders.mjs";
import { currentUser, json, paymentsMode } from "../../../../lib/session.mjs";

// Test-mode only: stands in for Stripe until test keys are configured.
export async function POST(request) {
  if (paymentsMode() !== "simulated") return json({ error: "Simulated payments are disabled." }, 403);
  const user = await currentUser();
  if (!user) return json({ error: "Sign in first." }, 401);
  const { order_id } = await request.json().catch(() => ({}));
  if (!orderFor(order_id, user.id)) return json({ error: "No such order." }, 404);
  const { order, error } = markPaid(order_id, { method: "simulated-test" });
  if (error) return json({ error }, 400);
  return json({ ok: true, status: order.status });
}
