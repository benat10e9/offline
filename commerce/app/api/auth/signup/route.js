import { cookies } from "next/headers";
import { COOKIE, cookieOptions, hashPassword, sessionValue } from "../../../../lib/auth.mjs";
import { json } from "../../../../lib/session.mjs";
import { newId, tx } from "../../../../lib/store.mjs";

export async function POST(request) {
  const { email, password } = await request.json().catch(() => ({}));
  const e = String(email || "").trim().toLowerCase();
  if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(e)) return json({ error: "Enter a valid email." }, 400);
  if (String(password || "").length < 10) return json({ error: "Use a password of at least 10 characters." }, 400);
  const user = tx((db) => {
    if (db.users.some((u) => u.email === e)) return null;
    const u = { id: newId("usr"), email: e, ...hashPassword(String(password)), created_at: new Date().toISOString() };
    db.users.push(u);
    return u;
  });
  if (!user) return json({ error: "An account with that email exists. Sign in instead." }, 409);
  (await cookies()).set(COOKIE, sessionValue(user.id), cookieOptions);
  return json({ ok: true });
}
