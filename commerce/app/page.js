import { EDITIONS, dollars } from "../lib/catalog.mjs";
import { paymentsMode } from "../lib/session.mjs";

export const dynamic = "force-dynamic";

export default function Home() {
  return (
    <main className="wrap">
      <h1>Answers when the internet is gone.</h1>
      <p className="lead">A USB drive with an AI assistant that runs on your own Mac or Windows computer, with no internet. Every answer cites the government manual or reference it came from, and says plainly when the library doesn't know.</p>
      {paymentsMode() === "simulated" && <p className="notice">Test store: payments are simulated. No card is charged.</p>}
      <div className="tiers">
        {Object.entries(EDITIONS).map(([id, e]) => (
          <div className="card" key={id}>
            <h2 style={{ margin: 0 }}>{e.name}</h2>
            <div className="price">{dollars(e.price_cents)}</div>
            <p className="muted">{e.blurb}</p>
            <p className="muted">Delivered on a {e.drive}.</p>
            <a className="btn" href={`/configure?edition=${id}`}>Configure</a>
          </div>
        ))}
      </div>
      <p className="muted" style={{ marginTop: 24 }}>
        Medical content is drawn from U.S. government publications and is reference material, not medical advice or a substitute for professional care. It has not been clinically reviewed for this product. In an emergency, call 911.
      </p>
    </main>
  );
}
