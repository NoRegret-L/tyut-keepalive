"""Verify the exact reviewed GLib backport and keep registry advisory coverage."""
import argparse
import hashlib
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parent.parent
APP = ROOT / "src-tauri"
VENDOR = APP / "vendor/glib-0.18.5"
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
ORIGINAL_SHA256 = "233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5"
# SHA-256 of sorted UTF-8 relative path + NUL + binary SHA-256(file bytes).
# Derived independently from the checksum-verified crates.io package plus
# only the official two-line change, not from the committed vendor tree.
PATCHED_SHA256 = "e8dbbd51b181d6ffc6d5e002bc28334a94760799a200181dedbfc263382076b1"

def require(condition, message):
    if not condition:
        raise SystemExit("GLib provenance check failed: " + message)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--audit-lock", type=Path)
    args = parser.parse_args()
    require(VENDOR.is_dir() and not VENDOR.is_symlink(), "missing vendor directory")
    digest = hashlib.sha256()
    paths = sorted(VENDOR.rglob("*"), key=lambda p: p.relative_to(VENDOR).as_posix())
    for path in paths:
        require(not path.is_symlink(), "unexpected symlink")
        if path.is_dir():
            continue
        require(path.is_file(), "unexpected special file")
        relative = path.relative_to(VENDOR).as_posix()
        digest.update(relative.encode("utf-8") + b"\0" + hashlib.sha256(path.read_bytes()).digest())
    require(digest.hexdigest() == PATCHED_SHA256, "vendor bytes differ from reviewed backport")
    manifest = tomllib.loads((APP / "Cargo.toml").read_text(encoding="utf-8"))
    require(manifest.get("patch", {}).get("crates-io", {}).get("glib") ==
            {"path": "vendor/glib-0.18.5"}, "expected local patch override")
    vendor_manifest = tomllib.loads((VENDOR / "Cargo.toml").read_text(encoding="utf-8"))
    require((vendor_manifest["package"]["name"], vendor_manifest["package"]["version"]) ==
            ("glib", "0.18.5"), "unexpected crate identity")
    lock_path = APP / "Cargo.lock"
    lock = lock_path.read_text(encoding="utf-8")
    packages = tomllib.loads(lock)["package"]
    glib = [p for p in packages if p["name"] == "glib"]
    require(len(glib) == 1 and glib[0]["version"] == "0.18.5" and
            "source" not in glib[0] and "checksum" not in glib[0], "expected one local GLib")
    if args.audit_lock is not None:
        target = args.audit_lock.resolve()
        require(target != lock_path.resolve(), "refusing to overwrite actual build lock")
        anchor = '[[package]]\nname = "glib"\nversion = "0.18.5"\n'
        require(lock.count(anchor) == 1, "unexpected GLib lock layout")
        restored = lock.replace(anchor, anchor + f'source = "{REGISTRY}"\nchecksum = "{ORIGINAL_SHA256}"\n', 1)
        expected = tomllib.loads(lock)
        next(p for p in expected["package"] if p["name"] == "glib").update(
            source=REGISTRY, checksum=ORIGINAL_SHA256)
        require(tomllib.loads(restored) == expected, "audit lock changed more than registry identity")
        target.write_text(restored, encoding="utf-8")
        print("Restored original registry identity for advisory lookup only; no warnings ignored.")
    print("Reviewed GLib 0.18.5 backport integrity OK.")

if __name__ == "__main__":
    main()
