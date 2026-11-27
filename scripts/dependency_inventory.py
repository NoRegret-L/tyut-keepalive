"""Collect reproducible graph evidence; no builds or lockfile changes."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "src-tauri/Cargo.toml"
LOCK = ROOT / "src-tauri/Cargo.lock"

def run(args, output):
    print("+", " ".join(args), flush=True)
    with output.open("wb") as f:
        subprocess.run(args, cwd=ROOT, stdout=f, check=True)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    lock_hash = hashlib.sha256(LOCK.read_bytes()).hexdigest()
    config = tomllib.loads((MANIFEST.parent / "deny.toml").read_text(encoding="utf-8"))
    about = tomllib.loads((MANIFEST.parent / "about.toml").read_text(encoding="utf-8"))
    targets = config["graph"]["targets"]
    if len(targets) != 4 or len(set(targets)) != 4 or targets != about["targets"]:
        raise SystemExit("four distinct targets must agree in both tool configs")
    for target in targets:
        folder = out / target
        folder.mkdir(exist_ok=True)
        common = ["--manifest-path", str(MANIFEST), "--locked"]
        run(["cargo", "metadata", *common, "--format-version", "1",
             "--filter-platform", target], folder / "metadata.json")
        for edges, filename in [
            ("normal", "runtime-tree.txt"),
            ("normal,build", "build-tree.txt"),
            ("normal,build,dev", "test-tree.txt"),
            ("normal,build,dev,features", "feature-tree.txt"),
        ]:
            run(["cargo", "tree", *common, "--target", target,
                 "--edges", edges, "--color", "never"], folder / filename)
    if hashlib.sha256(LOCK.read_bytes()).hexdigest() != lock_hash:
        raise SystemExit("Cargo.lock changed")
    npm_lock = json.loads((ROOT / "package-lock.json").read_text(encoding="utf-8"))
    npm = [{"path": path, "version": pkg.get("version"),
            "license": pkg.get("license"), "dev": pkg.get("dev", False),
            "optional": pkg.get("optional", False), "os": pkg.get("os"),
            "cpu": pkg.get("cpu"), "integrity": pkg.get("integrity")}
           for path, pkg in sorted(npm_lock["packages"].items()) if path]
    (out / "npm-lock-inventory.json").write_text(json.dumps(npm, indent=2) + "\n", encoding="utf-8")
    app = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))["package"]
    (out / "scope.json").write_text(json.dumps({
        "cargo_lock_sha256": lock_hash,
        "npm_lock_sha256": hashlib.sha256((ROOT / "package-lock.json").read_bytes()).hexdigest(),
        "targets": targets, "application_license": app.get("license"),
        "application_license_file": app.get("license-file"),
        "root_license_files": sorted(p.name for p in ROOT.glob("LICENSE*") if p.is_file()),
        "note": "Graph evidence, not exact binary linkage or distribution approval; default features."
    }, indent=2) + "\n", encoding="utf-8")

if __name__ == "__main__":
    main()
