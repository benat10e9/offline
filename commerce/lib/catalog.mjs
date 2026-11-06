// Product catalog. Prices are placeholders to be set by the business; sizes are measured
// (docs/editions.md). Package ids match the content/maps releases.
export const EDITIONS = {
  essential: {
    name: "Essential",
    price_cents: 5900,
    model: "qwen3-4b-q4km",
    packages: ["medical-survival"],
    maps: false,
    drive: "16 GB USB",
    blurb: "Medical, first aid, water, shelter and preparedness library with an offline AI assistant.",
  },
  standard: {
    name: "Standard",
    price_cents: 9900,
    model: "qwen3-8b-q4km",
    packages: ["medical-survival"],
    maps: true,
    drive: "16 GB USB",
    blurb: "Everything in Essential plus offline maps, place search and driving directions for your state.",
  },
  family: {
    name: "Family",
    price_cents: 14900,
    model: "qwen3-8b-q4km",
    packages: ["medical-survival", "wikipedia-top"],
    maps: true,
    drive: "16 GB USB",
    blurb: "Standard plus the 50,000 most-read Wikipedia articles for reference and schoolwork.",
  },
};

export const MODELS = {
  "qwen3-4b-q4km": { name: "Compact AI (4B)", min_ram_gb: 12, note: "Runs on most laptops with 12 GB or more memory." },
  "qwen3-8b-q4km": { name: "Standard AI (8B)", min_ram_gb: 16, note: "Better answers; needs 16 GB or more memory." },
};

// States with a built map package. Others are recorded as requests.
export const MAP_REGIONS = { "US-TX": "maps-texas" };

export const STATES = ["AL","AK","AZ","AR","CA","CO","CT","DE","DC","FL","GA","HI","ID","IL","IN","IA","KS","KY","LA","ME","MD","MA","MI","MN","MS","MO","MT","NE","NV","NH","NJ","NM","NY","NC","ND","OH","OK","OR","PA","RI","SC","SD","TN","TX","UT","VT","VA","WA","WV","WI","WY"];

// "linux" is accepted through the API (CI drives, power users) but not offered in the UI.
export const OS_CHOICES = ["macos", "windows", "linux"];

/** Validate a customer configuration; returns {config} or {error}. */
export function validateConfig(input, { edition } = {}) {
  const e = edition || input?.edition;
  if (!EDITIONS[e]) return { error: "Choose an edition." };
  const os = Array.isArray(input?.os) ? [...new Set(input.os.filter((o) => OS_CHOICES.includes(o)))] : [];
  if (!os.length) return { error: "Choose at least one computer type." };
  const st = String(input?.state || "").replace("US-", "");
  if (!STATES.includes(st)) return { error: "Choose your state." };
  const model = input?.model && MODELS[input.model] ? input.model : EDITIONS[e].model;
  const assets = (Array.isArray(input?.assets) ? input.assets : [])
    .slice(0, 25)
    .map((a) => ({
      label: String(a?.label || "").slice(0, 60).trim(),
      make: String(a?.make || "").slice(0, 60).trim(),
      model: String(a?.model || "").slice(0, 60).trim(),
      year: Number.isInteger(Number(a?.year)) && Number(a.year) > 1900 && Number(a.year) < 2100 ? Number(a.year) : null,
    }))
    .filter((a) => a.make && a.model);
  const grades = (Array.isArray(input?.grades) ? input.grades : []).map(String).filter((g) => /^(K|[1-9]|1[0-2])$/.test(g));
  return { config: { edition: e, os, state: `US-${st}`, model, assets, grades } };
}

/** The packages a configuration installs, plus requests we can't fulfil yet. */
export function resolvePackages(config) {
  const ed = EDITIONS[config.edition];
  const packages = [...ed.packages];
  const requests = [];
  if (ed.maps) {
    const pkg = MAP_REGIONS[config.state];
    if (pkg) packages.push(pkg);
    else requests.push(`map region ${config.state} (not yet available)`);
  }
  for (const a of config.assets) requests.push(`equipment documentation: ${a.make} ${a.model}${a.year ? ` (${a.year})` : ""}`);
  if (config.grades.length) requests.push(`school curriculum: grades ${config.grades.join(", ")}`);
  return { packages, requests };
}

export const dollars = (cents) => `$${(cents / 100).toFixed(2)}`;
