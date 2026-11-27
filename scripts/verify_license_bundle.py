# SPDX-License-Identifier: MPL-2.0
"""Use Tauri's resource-map mechanism and verify byte-exact DEB payload."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
def digest(p):
    with p.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()

def manifest(folder):
    entries = {}
    for line in (folder / "SHA256SUMS.txt").read_text(encoding="utf-8").splitlines():
        checksum, name = line.split("  ", 1)
        p = Path(name)
        if not re.fullmatch(r"[0-9a-f]{64}", checksum) or p.is_absolute() or ".." in p.parts or name in entries:
            raise SystemExit("invalid resource manifest")
        entries[name] = checksum
    actual = {p.relative_to(folder).as_posix(): digest(p) for p in folder.rglob("*") if p.is_file() and p.name != "SHA256SUMS.txt"}
    if not entries or actual != entries:
        raise SystemExit("license resource integrity mismatch")
    entries["SHA256SUMS.txt"] = digest(folder / "SHA256SUMS.txt")
    return entries

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--resources", type=Path, required=True)
    parser.add_argument("--deb", type=Path)
    args = parser.parse_args()
    entries = manifest(args.resources)
    if not args.deb:
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        identity = json.loads((args.resources / "SOURCE-ACCESS.json").read_text(encoding="utf-8"))
        if identity["commit"] != commit:
            raise SystemExit("resource source commit mismatch")
        stage = ROOT / "src-tauri/license-evidence"
        shutil.copytree(args.resources, stage)
        mapping = {f"license-evidence/{name}": f"licenses/{name}" for name in sorted(entries)}
        config = ROOT / "src-tauri/tauri.license-review.generated.json"
        if config.exists():
            raise SystemExit("refusing to replace existing config")
        config.write_text(json.dumps({"bundle": {"resources": mapping}}, indent=2) + "\n", encoding="utf-8")
        print("Generated an artifact-only Tauri resource overlay")
        return
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        subprocess.run(["dpkg-deb", "-x", str(args.deb.resolve()), temp], check=True)
        candidates = [p.parent for p in root.rglob("SHA256SUMS.txt") if p.parent.name == "licenses"]
        if len(candidates) != 1 or manifest(candidates[0]) != entries:
            raise SystemExit("DEB missing byte-exact license resources")
    print(f"DEB contains all {len(entries)} byte-exact license/source files.")

if __name__ == "__main__":
    main()
