"""Build a package source folder (package.json, sources.json, passages.jsonl, report.json)
from a rights catalog in catalog/sources/. `kb build` then indexes and signs it.

    python3 factory/build_package.py catalog/sources/medical-survival.json --work work --out out/medical-survival

Runs on build machines and in GitHub Actions only. It reads Kiwix ZIM archives with
python-libzim (GPL-3.0); that library never ships to customers (ADR 0003/0009).

HTML-to-text, PDF-to-text and passage splitting are adapted from survival-llm
(https://github.com/anonnerdfrenzy/survival-llm, MIT License, Copyright (c) 2026 Ronny Fernandez).
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.request

MIN_CHARS = 300
UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15"
KIWIX_CATALOG = "https://library.kiwix.org/catalog/v2/entries?lang=eng&count=5000"
KIWIX_MIRROR = "https://ftpmirror.your.org/pub/kiwix"

# ---------- text extraction (adapted from survival-llm, MIT) ----------

HEADINGS = {"h1": "# ", "h2": "## ", "h3": "### ", "h4": "#### ", "h5": "#### ", "h6": "#### "}
BLOCKS = {"p", "div", "section", "article", "ul", "ol", "dl", "dt", "dd", "table", "pre", "blockquote",
          "figure", "figcaption", "details", "summary", "main", "center", "hr", "caption"}
DROP_XPATH = (".//script|.//style|.//noscript|.//template|.//svg|.//button|.//form|.//nav|.//footer|.//header"
              "|.//aside|.//sup[contains(@class,'reference')]")
DROP_CLASS = re.compile(
    r"\b(mw-editsection|references|reflist|navbox|sidebar|metadata|noprint|catlinks|zim-footer|printfooter"
    r"|toc|breadcrumb|share|social|page-tools|syndicate)\b")
TITLE_SUFFIX = re.compile(r"\s*[|:\-–—]\s*(Ready\.gov|FEMA|CDC|Travelers' Health|MedlinePlus[^|]*)\s*$", re.I)


def node_text(node):
    """Element tree to plain text, keeping headings, numbered steps, bullets and table rows."""
    for bad in node.xpath(DROP_XPATH):
        bad.drop_tree()
    for el in node.xpath(".//*[@class]"):
        if DROP_CLASS.search(el.get("class") or ""):
            el.drop_tree()
    for ol in node.iter("ol"):
        for n, li in enumerate((c for c in ol if c.tag == "li"), 1):
            li.set("data-n", str(n))
    for el in node.iter():
        if not isinstance(el.tag, str):
            continue
        t = el.tag
        if t in HEADINGS:
            el.text = "\n\n" + HEADINGS[t] + (el.text or "")
            el.tail = "\n\n" + (el.tail or "")
        elif t == "li":
            el.text = (f"\n{el.get('data-n')}. " if el.get("data-n") else "\n- ") + (el.text or "")
        elif t == "tr":
            el.text = "\n" + (el.text or "")
        elif t in ("td", "th"):
            el.tail = " | " + (el.tail or "")
        elif t == "br":
            el.tail = "\n" + (el.tail or "")
        elif t in BLOCKS:
            el.text = "\n\n" + (el.text or "")
            el.tail = "\n\n" + (el.tail or "")
    lines = []
    for line in node.text_content().split("\n"):
        line = re.sub(r"[ \t\u00a0\u200b]+", " ", line).strip()
        line = re.sub(r"(\s*\|)+\s*$", "", line)
        if line in ("-", "#", "##", "###", "####") or re.fullmatch(r"\d+\.", line):
            line = ""
        lines.append(line)
    return re.sub(r"\n{3,}", "\n\n", "\n".join(lines)).strip()


def html_to_text(html):
    import lxml.html
    doc = lxml.html.fromstring(html)
    found = doc.xpath("//main") or doc.xpath("//article") or doc.xpath("//*[@id='content']") or doc.xpath("//body")
    return node_text(found[0]) if found else ""


def clean_title(title):
    return TITLE_SUFFIX.sub("", " ".join((title or "").split())).strip()


def pdf_to_text(path):
    r = subprocess.run(["pdftotext", "-q", "-enc", "UTF-8", path, "-"], capture_output=True, timeout=900)
    text = r.stdout.decode("utf-8", "replace").replace("\f", "\n\n")
    paras = [" ".join(p.split()) for p in re.split(r"\n\s*\n", text)]
    return "\n\n".join(p for p in paras if p)


def split_passages(text, target=1500, hard=3000):
    """Cut text into ~1,500-character passages on paragraph boundaries; returns [(heading, passage)]."""
    out, buf, size, heading, buf_heading = [], [], 0, "", ""
    for para in re.split(r"\n{2,}", text):
        para = para.strip()
        if not para:
            continue
        if para.startswith("#"):
            if buf and size > 200:
                out.append((buf_heading, "\n\n".join(buf)))
                buf, size = [], 0
            heading = para.lstrip("#").strip()
            if not buf:
                buf_heading = heading
            buf.append(para)
            size += len(para) + 2
            continue
        pieces = []
        while len(para) > hard:
            cut = para.rfind(". ", target // 2, hard)
            cut = cut + 1 if cut > 0 else hard
            pieces.append(para[:cut].strip())
            para = para[cut:].strip()
        pieces.append(para)
        for piece in pieces:
            if buf and size + len(piece) > target:
                out.append((buf_heading, "\n\n".join(buf)))
                buf, size, buf_heading = [], 0, heading
            buf.append(piece)
            size += len(piece) + 2
    if buf:
        out.append((buf_heading, "\n\n".join(buf)))
    return out

# ---------- acquisition ----------


def curl(url, dest):
    # Timeouts: a stalled mirror (e.g. the Internet Archive fallback) must fail, not hang the build.
    return subprocess.run(["curl", "-L", "--fail", "-sS", "-A", UA, "--retry", "4", "-C", "-",
                           "--connect-timeout", "30", "--speed-time", "120", "--speed-limit", "5000",
                           "-o", dest, url]).returncode == 0


def kiwix_urls():
    data = urllib.request.urlopen(KIWIX_CATALOG, timeout=180).read().decode()
    urls = {}
    for e in re.findall(r"<entry>(.*?)</entry>", data, re.S):
        name = re.search(r"<name>(.*?)</name>", e)
        flavour = re.search(r"<flavour>(.*?)</flavour>", e)
        link = re.search(r'rel="http://opds-spec.org/acquisition/open-access"[^>]*href="([^"]+)"', e) or \
            re.search(r'href="([^"]+\.zim\.meta4)"[^>]*rel="http://opds-spec.org/acquisition/open-access"', e)
        if name and link:
            urls[(name.group(1), flavour.group(1) if flavour else "")] = link.group(1).removesuffix(".meta4")
    return urls


def fetch_zim(src, work, urls):
    url = urls.get((src["zim"], src.get("flavour", "")))
    if not url:
        raise RuntimeError(f"{src['zim']} not in the Kiwix catalog")
    dest = os.path.join(work, url.rsplit("/", 1)[1])
    if os.path.exists(dest + ".done"):
        return dest, url
    for candidate in (re.sub(r"^https://([a-z]+\.)?download\.kiwix\.org", KIWIX_MIRROR, url), url):
        if curl(candidate, dest):
            try:
                want = urllib.request.urlopen(url + ".sha256", timeout=60).read().decode().split()[0]
                if sha256_file(dest) != want:
                    os.remove(dest)
                    continue
            except OSError:
                pass
            open(dest + ".done", "w").close()
            return dest, url
    raise RuntimeError(f"download failed: {url}")


def fetch_pdf(src, work):
    dest = os.path.join(work, src["id"] + ".pdf")
    if os.path.exists(dest) and open(dest, "rb").read(4) == b"%PDF":
        return dest
    archive = "https://web.archive.org/web/2026id_/" + src["url"]
    for u in filter(None, (src["url"], src.get("fallback"), archive)):
        if curl(u, dest) and open(dest, "rb").read(4) == b"%PDF":
            return dest
    raise RuntimeError(f"download failed: {src['url']}")


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 24):
            h.update(chunk)
    return h.hexdigest()

# ---------- documents ----------


class SourceErrors:
    """Counts per-page failures so a systematic error can't hide behind 'skipped' lines."""

    def __init__(self):
        self.count = 0
        self.first = None

    def add(self, exc):
        self.count += 1
        if self.first is None:
            self.first = repr(exc)


def zim_entry(arc, i):
    # libzim >= 3.2 made this public; older releases only had the underscore name.
    getter = getattr(arc, "get_entry_by_id", None) or getattr(arc, "_get_entry_by_id")
    return getter(i)


def zim_docs(path, skip, errors=None):
    from libzim.reader import Archive
    arc = Archive(path)
    skip_re = re.compile(skip) if skip else None
    errors = errors if errors is not None else SourceErrors()
    for i in range(arc.entry_count):
        try:
            e = zim_entry(arc, i)
            if e.is_redirect or (skip_re and skip_re.search(e.path)) or re.match(r"(?i)h?ttps?:", e.path):
                continue
            item = e.get_item()
            if item.mimetype.split(";")[0] != "text/html":
                continue
            text = html_to_text(bytes(item.content))
            if len(text) >= MIN_CHARS:
                yield clean_title(e.title or e.path), e.path, text
        except Exception as exc:  # one bad page must not stop the build
            errors.add(exc)
            if errors.count <= 5:
                print(f"    skipped entry {i}: {exc!r}", file=sys.stderr)


STEP_RE = re.compile(r"(?i)\bstep\s*(\d{1,2})\b")
FOOTER_RE = re.compile(r"(?i)\b(page\s+\d+(-\d+)?|(january|february|march|april|may|june|july|august|september|october|november|december)\s+\d{4})\b")


def truncated_procedure(text, next_text):
    """A numbered procedure that stops at a page footer and does not continue in the next
    passage: the remaining steps are probably in a figure the PDF text does not contain."""
    steps = [int(m.group(1)) for m in STEP_RE.finditer(text)]
    if not steps:
        return False
    tail = text[STEP_RE.finditer(text).__next__().start():]
    last = list(STEP_RE.finditer(text))[-1]
    after = text[last.end():]
    continues = any(int(m.group(1)) == max(steps) + 1 for m in STEP_RE.finditer(next_text or ""))
    return bool(tail) and len(after) < 300 and bool(FOOTER_RE.search(after)) and not continues


def doc_id(source_id, key):
    return f"{source_id}:{hashlib.sha1(key.encode()).hexdigest()[:12]}"


def passage_id(did, n, text):
    """Stable while the text is unchanged: document id + position + content hash."""
    return f"{did}:{n:04d}:{hashlib.sha1(text.encode()).hexdigest()[:8]}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("catalog")
    ap.add_argument("--work", default="work")
    ap.add_argument("--out", required=True)
    ap.add_argument("--only", nargs="*", help="source ids to include (testing)")
    ap.add_argument("--max-docs", type=int, default=0, help="cap documents per source (testing)")
    ap.add_argument("--require-all", action="store_true", help="exit non-zero if any source fails")
    args = ap.parse_args()

    cat = json.load(open(args.catalog))
    os.makedirs(args.work, exist_ok=True)
    os.makedirs(args.out, exist_ok=True)
    sources = [s for s in cat["sources"] if not args.only or s["id"] in args.only]
    urls = kiwix_urls() if any(s["kind"] == "zim" for s in sources) else {}

    records, report = [], {"package": cat["package"], "sources": {}, "failed": {}}
    seen_text = set()
    with open(os.path.join(args.out, "passages.jsonl"), "w") as out:
        for src in sources:
            print(f"== {src['id']}", flush=True)
            try:
                errors = SourceErrors()
                if src["kind"] == "zim":
                    path, url = fetch_zim(src, args.work, urls)
                    docs = zim_docs(path, src.get("skip"), errors)
                    origin = url
                else:
                    path = fetch_pdf(src, args.work)
                    text = pdf_to_text(path)
                    if len(text) < MIN_CHARS:
                        raise RuntimeError("no text layer (needs OCR)")
                    docs = [(src["title"], src["id"], text)]
                    origin = src["url"]
            except Exception as exc:
                print(f"   FAILED: {exc}", flush=True)
                report["failed"][src["id"]] = str(exc)
                continue
            ndocs = npass = chars = dupes = 0
            excluded = {}
            rules = [(re.compile(x["pattern"]), x["reason"]) for x in src.get("exclude_passages", [])]
            for title, key, text in docs:
                if args.max_docs and ndocs >= args.max_docs:
                    break
                did = doc_id(src["id"], key)
                ndocs += 1
                parts = split_passages(text)
                for i, (heading, ptxt) in enumerate(parts):
                    nxt = parts[i + 1][1] if i + 1 < len(parts) else ""
                    if truncated_procedure(ptxt, nxt):
                        report.setdefault("review_truncated_procedures", []).append(
                            {"source": src["id"], "title": title, "text_start": ptxt[:160]})
                for n, (heading, ptxt) in enumerate(parts):
                    rule = next((r for r in rules if r[0].search(ptxt)), None)
                    if rule:  # curated out, with the reason recorded in the report
                        excluded[rule[1]] = excluded.get(rule[1], 0) + 1
                        continue
                    digest = hashlib.sha1(ptxt.encode()).digest()
                    if digest in seen_text:  # identical boilerplate across pages
                        dupes += 1
                        continue
                    seen_text.add(digest)
                    out.write(json.dumps({
                        "passage_id": passage_id(did, n, ptxt), "doc_id": did, "source_id": src["id"],
                        "title": title, "heading": heading, "text": ptxt,
                    }, ensure_ascii=False) + "\n")
                    npass += 1
                    chars += len(ptxt)
            if ndocs == 0:
                why = f"0 documents extracted; {errors.count} page errors, first: {errors.first}"
                print(f"   FAILED: {why}", flush=True)
                report["failed"][src["id"]] = why
                continue
            if errors.count:
                print(f"   {errors.count} pages skipped with errors (first: {errors.first})", flush=True)
            records.append({
                "id": src["id"], "title": src["title"], "publisher": src["publisher"], "license": src["license"],
                "authority": src["authority"], "revision": src.get("revision"), "superseded_by": None,
                "applies_to": [], "jurisdictions": src.get("jurisdictions", ["US"]), "url": origin,
                **({"audience": src["audience"]} if src.get("audience") else {}),
            })
            report["sources"][src["id"]] = {"documents": ndocs, "passages": npass, "chars": chars,
                                            "duplicates_dropped": dupes, "page_errors": errors.count,
                                            "passages_excluded": excluded}
            print(f"   {ndocs} documents, {npass} passages, {chars / 1e6:.1f} M chars", flush=True)

    pkg = dict(cat["package"])
    pkg.setdefault("depends_on", [])
    json.dump(pkg, open(os.path.join(args.out, "package.json"), "w"), indent=1)
    json.dump(records, open(os.path.join(args.out, "sources.json"), "w"), indent=1)
    json.dump(report, open(os.path.join(args.out, "report.json"), "w"), indent=1)
    print(json.dumps(report, indent=1))
    if not records:
        sys.exit("no sources built")
    if args.require_all and report["failed"]:
        sys.exit(f"sources failed: {sorted(report['failed'])}")


if __name__ == "__main__":
    main()
