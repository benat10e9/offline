import { EDITIONS, MAP_REGIONS, MODELS, STATES } from "../../lib/catalog.mjs";
import ConfigureForm from "./ConfigureForm";

export default async function Configure({ searchParams }) {
  const sp = await searchParams;
  const edition = EDITIONS[sp?.edition] ? sp.edition : "essential";
  const initial = { edition, os: ["macos", "windows"], model: EDITIONS[edition].model, state: "TX", assets: [], grades: [], delivery: "usb" };
  return (
    <main className="wrap" style={{ maxWidth: 760 }}>
      <h1>Configure your offline library</h1>
      <ConfigureForm editions={EDITIONS} models={MODELS} states={STATES} mapRegions={MAP_REGIONS} initial={initial} />
    </main>
  );
}
