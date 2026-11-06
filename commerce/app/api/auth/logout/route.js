import { cookies } from "next/headers";
import { COOKIE } from "../../../../lib/auth.mjs";

export async function POST() {
  (await cookies()).delete(COOKIE);
  return Response.json({ ok: true });
}
