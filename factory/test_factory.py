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

    def test_catalog_is_commercially_safe(self):
        import json
        cat = json.load(open(os.path.join(os.path.dirname(__file__), "../catalog/sources/medical-survival.json")))
        for s in cat["sources"]:
            self.assertEqual(s["license"], "LicenseRef-PublicDomain-USGov", s["id"])
            self.assertIn(s["authority"], ("official", "government"))
        med = next(s for s in cat["sources"] if s["id"] == "medlineplus")
        self.assertIn("ency", med["skip"])
        self.assertIn("druginfo", med["skip"])


if __name__ == "__main__":
    unittest.main()
