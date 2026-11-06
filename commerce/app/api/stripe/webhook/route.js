import Stripe from "stripe";
import { markPaid } from "../../../../lib/orders.mjs";

// Stripe calls this after Checkout. The signature is verified against the raw body.
export async function POST(request) {
  const secret = process.env.STRIPE_WEBHOOK_SECRET;
  if (!process.env.STRIPE_SECRET_KEY || !secret) return new Response("not configured", { status: 503 });
  const stripe = new Stripe(process.env.STRIPE_SECRET_KEY);
  let event;
  try {
    event = stripe.webhooks.constructEvent(await request.text(), request.headers.get("stripe-signature"), secret);
  } catch {
    return new Response("bad signature", { status: 400 });
  }
  if (event.type === "checkout.session.completed" && event.data.object.payment_status === "paid") {
    const s = event.data.object;
    markPaid(s.metadata?.order_id, { method: "stripe", session: s.id, amount_total: s.amount_total, shipping: s.shipping_details || null });
  }
  return Response.json({ received: true });
}
