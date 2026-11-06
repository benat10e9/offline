import Stripe from "stripe";
import { EDITIONS } from "../../../lib/catalog.mjs";
import { createOrder } from "../../../lib/orders.mjs";
import { currentUser, json, paymentsMode } from "../../../lib/session.mjs";

export async function POST(request) {
  const user = await currentUser();
  if (!user) return json({ error: "Sign in first." }, 401);
  const body = await request.json().catch(() => ({}));
  const { order, error } = createOrder(user.id, body.config, body.delivery);
  if (error) return json({ error }, 400);
  const origin = process.env.PUBLIC_URL || new URL(request.url).origin;
  const mode = paymentsMode();
  if (mode === "stripe") {
    const stripe = new Stripe(process.env.STRIPE_SECRET_KEY);
    const session = await stripe.checkout.sessions.create({
      mode: "payment",
      customer_email: user.email,
      line_items: [{
        quantity: 1,
        price_data: { currency: "usd", unit_amount: order.amount_cents, product_data: { name: `Offline Library ${EDITIONS[order.config.edition].name}` } },
      }],
      metadata: { order_id: order.id },
      shipping_address_collection: order.delivery === "usb" ? { allowed_countries: ["US"] } : undefined,
      success_url: `${origin}/portal?paid=${order.id}`,
      cancel_url: `${origin}/configure?edition=${order.config.edition}`,
    });
    return json({ order_id: order.id, checkout_url: session.url });
  }
  if (mode === "simulated") return json({ order_id: order.id, checkout_url: `/checkout/test?order=${order.id}` });
  return json({ error: "Payments are not configured." }, 503);
}
