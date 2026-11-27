import base64
import copy
import unittest
from check_update_manifest import check

class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.tag = "v0.4.0"
        self.data = {"version": "0.4.0", "platforms": {}}
        self.assets = []
        sig = base64.b64encode(b"trusted comment: timestamp:0\tfile:fixture\tversion:0.4.0\n").decode()
        for platform, name in [("windows-x86_64", "app.msi"), ("darwin-aarch64", "app.app.tar.gz"), ("darwin-x86_64", "app.app.tar.gz"), ("linux-x86_64", "app.AppImage")]:
            self.data["platforms"][platform] = {
                "signature": sig,
                "url": "https://github.com/NoRegret-L/tyut-keepalive/releases/download/v0.4.0/" + name
            }
            self.assets.extend([{"name": name, "size": 100}, {"name": name + ".sig", "size": 10}])

    def test_complete_manifest(self):
        check(self.data, self.tag, self.assets)

    def test_missing_platform(self):
        del self.data["platforms"]["darwin-aarch64"]
        with self.assertRaises(SystemExit):
            check(self.data, self.tag, self.assets)

    def test_missing_intel_mac(self):
        del self.data["platforms"]["darwin-x86_64"]
        with self.assertRaises(SystemExit):
            check(self.data, self.tag, self.assets)

    def test_fork_asset_rejected(self):
        item = self.data["platforms"]["windows-x86_64"]
        item["url"] = item["url"].replace("NoRegret-L/", "Code-famer-horse/")
        with self.assertRaises(SystemExit):
            check(self.data, self.tag, self.assets)

    def test_missing_signature_asset(self):
        with self.assertRaises(SystemExit):
            check(self.data, self.tag, self.assets[:-1])

    def test_wrong_platform_asset(self):
        self.data["platforms"]["darwin-aarch64"]["url"] = self.data["platforms"]["windows-x86_64"]["url"]
        with self.assertRaises(SystemExit):
            check(self.data, self.tag, self.assets)

    def test_wrong_signed_version(self):
        self.data["platforms"]["linux-x86_64"]["signature"] = base64.b64encode(b"trusted comment: timestamp:0\tversion:0.3.3").decode()
        with self.assertRaises(SystemExit):
            check(self.data, self.tag, self.assets)

if __name__ == "__main__":
    unittest.main()
