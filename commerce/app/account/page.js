"use client";
import { useState } from "react";

export default function Account() {
  const [mode, setMode] = useState("signup");
  const [msg, setMsg] = useState("");
  async function submit(e) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    const r = await fetch(`/api/auth/${mode}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ email: f.get("email"), password: f.get("password") }) });
    const d = await r.json();
    if (!r.ok) return setMsg(d.error || "Something went wrong.");
    const next = new URLSearchParams(location.search).get("next") || "/portal";
    location.href = next;
  }
  return (
    <main className="wrap" style={{ maxWidth: 460 }}>
      <h1>{mode === "signup" ? "Create your account" : "Sign in"}</h1>
      <form onSubmit={submit} style={{ display: "grid", gap: 12 }}>
        <label>Email<input name="email" type="email" autoComplete="email" required /></label>
        <label>Password<input name="password" type="password" minLength={mode === "signup" ? 10 : 1} autoComplete={mode === "signup" ? "new-password" : "current-password"} required /></label>
        <button className="btn" type="submit">{mode === "signup" ? "Create account" : "Sign in"}</button>
        {msg && <p className="err" role="alert">{msg}</p>}
      </form>
      <p className="muted">
        {mode === "signup" ? "Already have an account? " : "New here? "}
        <button className="btn secondary" type="button" onClick={() => { setMode(mode === "signup" ? "login" : "signup"); setMsg(""); }}>
          {mode === "signup" ? "Sign in" : "Create an account"}
        </button>
      </p>
    </main>
  );
}
