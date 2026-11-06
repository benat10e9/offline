"use client";
import { useState } from "react";

export default function TestCheckout() {
  const [msg, setMsg] = useState("");
  async function pay() {
    const order_id = new URLSearchParams(location.search).get("order");
    const r = await fetch("/api/checkout/mock", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ order_id }) });
    const d = await r.json();
    if (!r.ok) return setMsg(d.error || "Payment failed.");
    location.href = `/portal?paid=${order_id}`;
  }
  return (
    <main className="wrap" style={{ maxWidth: 560 }}>
      <h1>Test payment</h1>
      <p className="notice">Stripe test keys aren't configured, so this store simulates payment. No card is charged.</p>
      <button className="btn" onClick={pay}>Simulate a successful payment</button>
      {msg && <p className="err" role="alert">{msg}</p>}
    </main>
  );
}
