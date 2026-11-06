import { orderFor } from "../../../../../lib/orders.mjs";
import { currentUser, json } from "../../../../../lib/session.mjs";

// GET ?v=<version>&file=json|sig  -> the signed profile files for the drive builder.
export async function GET(request, { params }) {
  const user = await currentUser();
  if (!user) return json({ error: "Sign in first." }, 401);
  const { id } = await params;
  const order = orderFor(id, user.id);
  if (!order) return json({ error: "No such order." }, 404);
  const url = new URL(request.url);
  const v = Number(url.searchParams.get("v")) || order.profiles.length;
  const p = order.profiles.find((x) => x.version === v);
  if (!p) return json({ error: "No such profile version." }, 404);
  const file = url.searchParams.get("file");
  if (file === "json" || file === "sig") {
    return new Response(file === "json" ? p.profile_json : p.profile_sig, {
      headers: {
        "content-type": file === "json" ? "application/json" : "text/plain",
        "content-disposition": `attachment; filename="profile.${file}"`,
      },
    });
  }
  return json({ version: p.version, profile_json: p.profile_json, profile_sig: p.profile_sig });
}
