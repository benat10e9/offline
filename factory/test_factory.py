import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import build_package as bp  # noqa: E402

HTML = b"""<html><body><header>Site nav</header><main>
<h1>Heat Exhaustion</h1><p>Move the person to a cooler place.</p>
<ol><li>Loosen clothing.</li><li>Give sips of water.</li></ol>
<div class="share">Share this page</div><script>track()</script>
</main><footer>Footer</footer></body></html>"""


class TextTests(unittest.TestCase):
    def test_html_keeps_structure_and_drops_chrome(self):
        t = bp.html_to_text(HTML)
        self.assertIn("# Heat Exhaustion", t)
        self.assertIn("1. Loosen clothing.", t)
        self.assertIn("2. Give sips of water.", t)
        for junk in ("Site nav", "Footer", "Share this page", "track()"):
            self.assertNotIn(junk, t)

    def test_split_respects_size_and_headings(self):
        text = "# Burns\n\n" + "\n\n".join("Cool the burn under running water for twenty minutes. " * 6 for _ in range(10))
        parts = bp.split_passages(text)
        self.assertGreater(len(parts), 1)
        self.assertTrue(all(len(p) <= 3200 for _, p in parts))
        self.assertEqual(parts[0][0], "Burns")

    def test_ids_are_stable_and_content_addressed(self):
        d = bp.doc_id("medlineplus", "medlineplus.gov/heatillness.html")
        self.assertEqual(d, bp.doc_id("medlineplus", "medlineplus.gov/heatillness.html"))
        self.assertNotEqual(bp.passage_id(d, 0, "a"), bp.passage_id(d, 0, "b"))

    def test_catalogs_use_commercial_licenses_only(self):
        import glob
        import json
        ok = {"LicenseRef-PublicDomain-USGov", "CC-BY-SA-4.0", "CC-BY-4.0", "ODbL-1.0"}
        for path in glob.glob(os.path.join(os.path.dirname(__file__), "../catalog/sources/*.json")):
            for s in json.load(open(path))["sources"]:
                self.assertIn(s["license"], ok, f"{path}: {s['id']}")
                self.assertNotIn("NC", s["license"])

    def test_catalog_is_commercially_safe(self):
        import json
        cat = json.load(open(os.path.join(os.path.dirname(__file__), "../catalog/sources/medical-survival.json")))
        for s in cat["sources"]:
            self.assertEqual(s["license"], "LicenseRef-PublicDomain-USGov", s["id"])
            self.assertIn(s["authority"], ("official", "government"))
        med = next(s for s in cat["sources"] if s["id"] == "medlineplus")
        self.assertIn("ency", med["skip"])
        self.assertIn("druginfo", med["skip"])


class ZimTests(unittest.TestCase):
    def test_reads_a_real_zim(self):
        try:
            from libzim.writer import Creator, Hint, Item, StringProvider
        except ImportError:
            self.skipTest("libzim not installed")
        import tempfile

        class Page(Item):
            def __init__(self, path, title, html):
                super().__init__()
                self.path, self.title, self.html = path, title, html

            def get_path(self):
                return self.path

            def get_title(self):
                return self.title

            def get_mimetype(self):
                return "text/html"

            def get_contentprovider(self):
                return StringProvider(self.html)

            def get_hints(self):
                return {Hint.FRONT_ARTICLE: True}

        body = "<p>" + "Drink water slowly and rest in the shade. " * 12 + "</p>"
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "t.zim")
            with Creator(path).config_indexing(False, "eng") as c:
                c.set_mainpath("heat.html")
                for k, v in {"Title": "t", "Language": "eng", "Name": "t", "Creator": "t", "Publisher": "t",
                             "Date": "2026-11-05", "Description": "t"}.items():
                    c.add_metadata(k, v)
                c.add_item(Page("heat.html", "Heat Illness | MedlinePlus", f"<main><h1>Heat</h1>{body}</main>"))
                c.add_item(Page("ency/article/1.html", "Licensed", f"<main>{body}</main>"))
                c.add_item(Page("medlineplus.gov/druginfo/meds/a1.html", "Licensed drug", f"<main>{body}</main>"))
            errs = bp.SourceErrors()
            import json
            cat = json.load(open(os.path.join(os.path.dirname(__file__), "../catalog/sources/medical-survival.json")))
            skip = next(x["skip"] for x in cat["sources"] if x["id"] == "medlineplus")
            docs = list(bp.zim_docs(path, skip, errs))
            self.assertEqual(errs.count, 0, errs.first)
            self.assertEqual([t for t, _, _ in docs], ["Heat Illness"], "licensed section skipped, title cleaned")


if __name__ == "__main__":
    unittest.main()
