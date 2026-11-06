import "./globals.css";

export const metadata = { title: "Offline Library", description: "An offline AI reference library on a USB drive." };

export default function RootLayout({ children }) {
  return (
    <html lang="en">
      <body>
        <header className="top">
          <a href="/" style={{ fontWeight: 700, textDecoration: "none", color: "inherit" }}>Offline Library</a>
          <nav><a href="/portal">My orders</a><a href="/account">Account</a></nav>
        </header>
        {children}
      </body>
    </html>
  );
}
