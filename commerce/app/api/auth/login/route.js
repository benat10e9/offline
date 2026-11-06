import { cookies } from "next/headers";
import { COOKIE, checkPassword, cookieOptions, sessionValue } from "../../../../lib/auth.mjs";
import { json } from "../../../../lib/session.mjs";
import { read } from "../../../../lib/store.mjs";

export async function POST(request) {
  const { email, password } = await request.json().catch(() => ({}));
  const u = read((db) => db.users.find((x) => x.email === String(email || "").trim().toLowerCase()));
  if (!u || !checkPassword(String(password || ""), u.salt, u.hash)) return json({ error: "Email or password is incorrect." }, 401);
  (await cookies()).set(COOKIE, sessionValue(u.id), cookieOptions);
  return json({ ok: true });
}
