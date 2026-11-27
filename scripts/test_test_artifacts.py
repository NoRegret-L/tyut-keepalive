import io
import tarfile
import unittest
from collect_test_artifacts import verify_deb_members
from prepare_test_build import isolate_linux_package

def archive(entries):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as tar:
        for name, mode in entries:
            member = tarfile.TarInfo(name)
            member.mode = mode
            member.size = 1
            tar.addfile(member, io.BytesIO(b"x"))
    buf.seek(0)
    return tarfile.open(fileobj=buf, mode="r|")

class TestArtifactChecks(unittest.TestCase):
    def test_normalized_paths(self):
        for name in ["./usr/bin/tyut-keepalive-test", "usr/bin/tyut-keepalive-test"]:
            with archive([(name, 0o755)]) as tar:
                verify_deb_members(tar)

    def test_reject_production_path(self):
        with archive([("usr/bin/tyut-keepalive-test", 0o755), ("usr/bin/tyut-keepalive", 0o755)]) as tar:
            with self.assertRaises(SystemExit):
                verify_deb_members(tar)

    def test_reject_suffix_and_nonexecutable(self):
        for name, mode in [("usr/bin/tyut-keepalive-test.old", 0o755), ("usr/bin/tyut-keepalive-test", 0o644)]:
            with archive([(name, mode)]) as tar:
                with self.assertRaises(SystemExit):
                    verify_deb_members(tar)

    def test_only_root_identity_changes(self):
        manifest = '[package]\nname = "tyut-keepalive"\nversion = "0.3.3"\n[lib]\nname = "tyut_keepalive_lib"\n'
        lock = 'version = 4\n[[package]]\nname = "tyut-keepalive"\nversion = "0.3.3"\ndependencies = ["serde"]\n[[package]]\nname = "serde"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n'
        updated_manifest, updated_lock = isolate_linux_package(manifest, lock)
        self.assertEqual(updated_manifest, manifest.replace('name = "tyut-keepalive"', 'name = "tyut-keepalive-test"'))
        self.assertEqual(updated_lock, lock.replace('name = "tyut-keepalive"', 'name = "tyut-keepalive-test"'))

    def test_reject_unexpected_identity(self):
        with self.assertRaises(SystemExit):
            isolate_linux_package('[package]\nname = "other"\n', 'version = 4\n')

if __name__ == "__main__":
    unittest.main()
