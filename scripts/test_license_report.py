# SPDX-License-Identifier: MPL-2.0
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from stage_license_notices import report_sections, render_report, check_report, copy_crate_notices

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

class CrateNoticeTests(unittest.TestCase):
    def test_exact_selectors_source_header_exception(self):
        package = {"name": "selectors", "version": "0.38.0", "license": "MPL-2.0"}
        header = (b"/* This Source Code Form is subject to the terms of the Mozilla Public\n"
                  b" * License, v. 2.0. If a copy of the MPL was not distributed with this\n"
                  b" * file, You can obtain one at https://mozilla.org/MPL/2.0/. */")
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / "selectors.crate"
            for version, text, ok in (("0.38.0", header, True),
                                      ("0.38.0", b"// missing header", False),
                                      ("0.38.1", header, False)):
                with tarfile.open(archive, "w:gz") as tar:
                    content = text + b"\nfn example() {}"
                    info = tarfile.TarInfo(f"selectors-{version}/lib.rs")
                    info.size = len(content)
                    tar.addfile(info, io.BytesIO(content))
                with tempfile.TemporaryDirectory(dir=root) as dest:
                    output = Path(dest)
                    record = {**package, "version": version}
                    if ok:
                        result = copy_crate_notices(archive, record, output)
                        self.assertEqual(result["original_root_notice_files"], 0)
                        self.assertEqual((output / "SOURCE-HEADER-lib.rs.txt").read_bytes(), header + b"\n")
                        self.assertTrue((output / "MPL-2.0-standard-supplied.txt").is_file())
                    else:
                        with self.assertRaises(SystemExit):
                            copy_crate_notices(archive, record, output)

if __name__ == "__main__":
    unittest.main()
