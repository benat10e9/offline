import { redirect } from "next/navigation";
import { EDITIONS, MODELS, dollars } from "../../lib/catalog.mjs";
import { ordersFor } from "../../lib/orders.mjs";
import { currentUser } from "../../lib/session.mjs";

export const dynamic = "force-dynamic";

const STATUS = { queued: "Queued for building", building: "Building", ready: "Ready", shipped: "Shipped", failed: "Build failed (we're on it)" };

export default async function Portal({ searchParams }) {
  const user = await currentUser();
  if (!user) redirect("/account?next=/portal");
  const sp = await searchParams;
  const orders = ordersFor(user.id);
  return (
    <main className="wrap">
      <h1>My orders</h1>
      {sp?.paid && <p className="ok">Payment received. Your drive is queued for building.</p>}
      {sp?.updated && <p className="ok">Configuration saved. An updated build is queued.</p>}
      {!orders.length && <p>No orders yet. <a href="/">Choose an edition</a>.</p>}
      {orders.map((o) => {
        const latest = o.profiles[o.profiles.length - 1];
        const p = latest ? JSON.parse(latest.profile_json) : null;
        return (
          <section className="card" key={o.id} style={{ marginBottom: 16 }}>
            <h2 style={{ marginTop: 0 }}>{EDITIONS[o.config.edition].name} <span className="pill">{o.status.replace("_", " ")}</span></h2>
            <p className="muted">Order {o.id} · {dollars(o.amount_cents)} · {o.delivery === "usb" ? "USB drive" : "Download"} · {MODELS[o.config.model].name} · {o.config.os.join(", ")} · {o.config.state}</p>
            {p && (
              <>
                <p>Installs: {p.packages.join(", ")}</p>
                {p.requests.length > 0 && <p className="muted">Requested for future updates: {p.requests.join("; ")}</p>}
                <table>
                  <thead><tr><th>Configuration</th><th>Build</th><th>Signed profile</th></tr></thead>
                  <tbody>
                    {o.builds.map((b) => (
                      <tr key={b.id}>
                        <td>Version {b.profile_version}</td>
                        <td>{STATUS[b.status] || b.status}{b.note ? `: ${b.note}` : ""}</td>
                        <td><a href={`/api/orders/${o.id}/profile?v=${b.profile_version}&file=json`}>profile.json</a> · <a href={`/api/orders/${o.id}/profile?v=${b.profile_version}&file=sig`}>profile.sig</a></td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                <p><a className="btn secondary" href={`/portal/${o.id}/edit`}>Change configuration</a></p>
              </>
            )}
            {o.status === "pending_payment" && <p className="muted">Waiting for payment.</p>}
          </section>
        );
      })}
    </main>
  );
}
