import { notFound, redirect } from "next/navigation";
import { EDITIONS, MAP_REGIONS, MODELS, STATES } from "../../../../lib/catalog.mjs";
import { orderFor } from "../../../../lib/orders.mjs";
import { currentUser } from "../../../../lib/session.mjs";
import ConfigureForm from "../../../configure/ConfigureForm";

export const dynamic = "force-dynamic";

export default async function Edit({ params }) {
  const user = await currentUser();
  const { id } = await params;
  if (!user) redirect(`/account?next=/portal/${id}/edit`);
  const o = orderFor(id, user.id);
  if (!o || o.status !== "paid") notFound();
  const initial = { ...o.config, state: o.config.state.replace("US-", ""), assets: o.config.assets.map((a) => ({ ...a, year: a.year || "" })) };
  return (
    <main className="wrap" style={{ maxWidth: 760 }}>
      <h1>Change configuration</h1>
      <p className="muted">Saving issues a new signed profile and queues an updated build. Your existing drive keeps working.</p>
      <ConfigureForm editions={EDITIONS} models={MODELS} states={STATES} mapRegions={MAP_REGIONS} initial={initial} orderId={o.id} />
    </main>
  );
}
