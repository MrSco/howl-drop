"""tools/test-adapter.ps1 knows the MT7612U by the USB IDs the driver takes."""

import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent


class TestAdapterScript(unittest.TestCase):
    def test_ids_match_the_driver(self):
        lib = (ROOT / "crates/mt7612u/src/lib.rs").read_text(encoding="utf-8")
        table = lib.split("pub const USB_IDS", 1)[1].split("];", 1)[0]
        driver = {f"{v}:{p}" for v, p in re.findall(r"\(0x([0-9a-f]{4}), 0x([0-9a-f]{4})\)", table)}
        script = (ROOT / "tools/test-adapter.ps1").read_text(encoding="ascii")
        ids = script.split("$Mt7612uIds = @(", 1)[1].split(")", 1)[0]
        self.assertTrue(driver)
        self.assertEqual(set(re.findall(r"'([0-9a-f]{4}:[0-9a-f]{4})'", ids)), driver)

    def test_plain_ascii(self):
        # Windows PowerShell reads `irm | iex` text as Latin-1.
        (ROOT / "tools/test-adapter.ps1").read_bytes().decode("ascii")

    def test_never_exits(self):
        # exit would close the window the line was pasted into.
        script = (ROOT / "tools/test-adapter.ps1").read_text(encoding="ascii")
        code = "\n".join(l for l in script.splitlines() if not l.lstrip().startswith("#"))
        # As a statement: at a line's start, or after { or ; (text may say it).
        self.assertNotRegex(code, r"(?im)(^|[{;])\s*exit\b")

    def test_form_fields_the_script_fills(self):
        form = (ROOT / ".github/ISSUE_TEMPLATE/adapter-report.yml").read_text(encoding="utf-8")
        script = (ROOT / "tools/test-adapter.ps1").read_text(encoding="ascii")
        for field in re.findall(r"&(\w+)=", script):
            if field != "title":
                self.assertIn(f"id: {field}\n", form)
        self.assertIn("template=adapter-report.yml", script)


if __name__ == "__main__":
    unittest.main()
