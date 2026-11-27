"""Prepare an isolated, checksum-verified GLib backport experiment.

Never run this against the working checkout. No production dependency changes.
"""
import argparse
import copy
import difflib
import hashlib
import json
from pathlib import Path
import re
import tarfile
import tomllib
import urllib.request

VERSION = "0.18.5"
SHA256 = "233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5"
SOURCE = "https://static.crates.io/crates/glib/glib-0.18.5.crate"
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"

def require(condition, message):
    if not condition:
        raise SystemExit(message)

def replace_once(source, old, new):
    require(source.count(old) == 1, f"unexpected patch context: {old!r}")
    return source.replace(old, new, 1)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    root = args.checkout.resolve()
    evidence = args.evidence.resolve()
    require(root != Path(__file__).resolve().parents[1], "refusing to modify working checkout")
    require(not (root / ".git").exists(), "use a disposable git archive, not a git checkout")
    evidence.mkdir(parents=True, exist_ok=False)
    app = root / "src-tauri"
    manifest_path = app / "Cargo.toml"
    lock_path = app / "Cargo.lock"
    lock = lock_path.read_text(encoding="utf-8")
    parsed = tomllib.loads(lock)
    glibs = [p for p in parsed["package"] if p["name"] == "glib"]
    require(len(glibs) == 1, "expected one GLib version")
    glib = glibs[0]
    require((glib["version"], glib.get("source"), glib.get("checksum")) ==
            (VERSION, REGISTRY, SHA256), "GLib lock identity changed; re-review first")
    # The original crates.io artifact is authenticated by the reviewed lock checksum.
    archive = evidence / f"glib-{VERSION}.crate"
    with urllib.request.urlopen(SOURCE, timeout=60) as response:
        archive.write_bytes(response.read())
    require(hashlib.file_digest(archive.open("rb"), "sha256").hexdigest() == SHA256,
            "crate SHA-256 mismatch")
    vendor = app / "vendor"
    extracted = vendor / f"glib-{VERSION}"
    require(not extracted.exists(), "vendor destination already exists")
    with tarfile.open(archive, "r:gz") as tar:
        members = tar.getmembers()
        for member in members:
            require(Path(member.name).parts[:1] == (f"glib-{VERSION}",)
                    and (member.isfile() or member.isdir()), "unexpected archive member")
        tar.extractall(vendor, members=members, filter="data")
    upstream_manifest = tomllib.loads((extracted / "Cargo.toml").read_text(encoding="utf-8"))
    require(upstream_manifest["package"]["name"] == "glib" and
            upstream_manifest["package"]["version"] == VERSION, "crate identity mismatch")
    require(any(p.is_file() for p in extracted.glob("LICENSE*")), "missing upstream license")
    path = extracted / "src/variant_iter.rs"
    before = path.read_text(encoding="utf-8")
    # gtk-rs/gtk-rs-core#1343; same two replacements as Debian 0.18.5-7 patch 0007.
    after = replace_once(before, "let p: *mut libc::c_char = std::ptr::null_mut();",
                         "let mut p: *mut libc::c_char = std::ptr::null_mut();")
    after = replace_once(after, "\n                s as *const u8 as *const _,\n                &p,\n                std::ptr::null::<i8>(),\n",
                         "\n                s as *const u8 as *const _,\n                &mut p,\n                std::ptr::null::<i8>(),\n")
    path.write_text(after, encoding="utf-8")
    patch = "".join(difflib.unified_diff(before.splitlines(keepends=True),
                    after.splitlines(keepends=True), fromfile="a/src/variant_iter.rs",
                    tofile="b/src/variant_iter.rs"))
    (evidence / "upstream-backport.diff").write_text(patch, encoding="utf-8")

    # Only the disposable copy is patched. Keep all resolved crate versions.
    manifest = manifest_path.read_text(encoding="utf-8")
    manifest = replace_once(manifest, "[patch.crates-io]\n",
                            '[patch.crates-io]\nglib = { path = "vendor/glib-0.18.5" }\n')
    manifest += '\n[target.\'cfg(target_os = "linux")\'.dev-dependencies]\nglib = "=0.18.5"\n'
    tomllib.loads(manifest)
    blocks = lock.split("[[package]]")
    glib_index = [i for i, s in enumerate(blocks)
                  if s.startswith('\nname = "glib"\nversion = "0.18.5"\n')]
    app_index = [i for i, s in enumerate(blocks)
                 if s.startswith('\nname = "tyut-keepalive"\nversion = "0.5.0"\n')]
    require(len(glib_index) == len(app_index) == 1, "unexpected lock layout")
    i, j = glib_index[0], app_index[0]
    blocks[i] = replace_once(blocks[i], f'source = "{REGISTRY}"\nchecksum = "{SHA256}"\n', "")
    match = re.search(r"dependencies = \[\n(.*?)\n\]", blocks[j], re.S)
    require(match is not None, "root dependencies missing")
    old_deps = match.group(1)
    deps = re.findall(r'^ "([^"]+)",$', old_deps, re.M)
    require(len(deps) == len(old_deps.splitlines()) and "glib" not in deps,
            "unexpected root dependency format")
    new_deps = "\n".join(f' "{dep}",' for dep in sorted(deps + ["glib"]))
    blocks[j] = replace_once(blocks[j], old_deps, new_deps)
    patched_lock = "[[package]]".join(blocks)
    expected = copy.deepcopy(parsed)
    for pkg in expected["package"]:
        if pkg["name"] == "glib":
            pkg.pop("source")
            pkg.pop("checksum")
        if pkg["name"] == "tyut-keepalive" and "source" not in pkg:
            pkg["dependencies"] = sorted(pkg["dependencies"] + ["glib"])
    require(tomllib.loads(patched_lock) == expected, "unexpected dependency resolution change")
    manifest_path.write_text(manifest, encoding="utf-8")
    lock_path.write_text(patched_lock, encoding="utf-8")
    tests = app / "tests"
    tests.mkdir(exist_ok=True)
    dest = tests / "glib_variant_str_iter_probe.rs"
    require(not dest.exists(), "test file already exists")
    dest.write_bytes((root / "scripts/glib_variant_str_iter_probe.rs").read_bytes())
    (evidence / "patched-Cargo.lock").write_text(patched_lock, encoding="utf-8")
    (evidence / "provenance.json").write_text(json.dumps({
        "advisory": "RUSTSEC-2024-0429",
        "source": SOURCE, "source_sha256": SHA256,
        "upstream_fix": "https://github.com/gtk-rs/gtk-rs-core/pull/1343",
        "debian_backport": "https://sources.debian.org/patches/rust-glib-0.18/0.18.5-7/0007-glib-fix-UB-in-VariantStrIter-impl_get.patch/",
        "scope": "disposable Linux CI checkout only"
    }, indent=2) + "\n", encoding="utf-8")
    print("Verified original crate checksum; applied only official two-line fix.")

if __name__ == "__main__":
    main()
