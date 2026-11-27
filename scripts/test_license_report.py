# SPDX-License-Identifier: MPL-2.0
import unittest
from stage_license_notices import report_sections, render_report, check_report

def entry(ident, body, name="unicode-ident", version="1.0.27"):
    return {"id": ident, "name": ident, "text": body,
            "used_by": [{"crate": {"name": name, "version": version}}]}

class LicenseReportTests(unittest.TestCase):
    def test_delimiter_inside_original_notice(self):
        body = "Copyright example\n" + "=" * 70 + "\nOriginal notice continues"
        report = {"licenses": [entry("Unicode-3.0", body), entry("MIT", "MIT terms")]}
        sections = report_sections(report)
        self.assertIn(("Unicode-3.0", body), sections["unicode-ident 1.0.27"])
        self.assertIn(body, render_report(report))
        check_report(sections, {"name": "unicode-ident", "version": "1.0.27", "license": "Unicode-3.0"})

    def test_missing_conjunct_fails(self):
        sections = report_sections({"licenses": [entry("Unicode-3.0", "Unicode terms")]})
        with self.assertRaises(SystemExit):
            check_report(sections, {"name": "unicode-ident", "version": "1.0.27", "license": "Unicode-3.0"})

    def test_exact_version_and_exception_text_required(self):
        p = {"name": "target-lexicon", "version": "0.12.16", "license": "Apache-2.0 WITH LLVM-exception"}
        for data in (entry("Apache-2.0", "Apache terms", p["name"], p["version"]),
                     entry("Apache-2.0", "LLVM exception", p["name"], "0.12.15")):
            with self.assertRaises(SystemExit):
                check_report(report_sections({"licenses": [data]}), p)
        check_report(report_sections({"licenses": [entry("Apache-2.0", "Apache terms\nLLVM exception", p["name"], p["version"])]}), p)

    def test_empty_text_fails(self):
        with self.assertRaises(SystemExit):
            report_sections({"licenses": [entry("MIT", "")]})

if __name__ == "__main__":
    unittest.main()
