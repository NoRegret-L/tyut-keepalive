"""Exclude only the application from third-party notices, in a disposable archive."""
import argparse
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--checkout", type=Path, required=True)
    args = parser.parse_args()
    checkout = args.checkout.resolve()
    if checkout == ROOT or (checkout / ".git").exists():
        raise SystemExit("use a disposable git archive, not the working checkout")
    manifest = checkout / "src-tauri/Cargo.toml"
    source = manifest.read_text(encoding="utf-8")
    original = tomllib.loads(source)
    if original["package"]["name"] != "tyut-keepalive":
        raise SystemExit("unexpected application package")
    if "publish" not in original["package"]:
        if source.count("[package]\n") != 1:
            raise SystemExit("unexpected manifest layout")
        source = source.replace("[package]\n", "[package]\npublish = false\n", 1)
        original["package"]["publish"] = False
        if tomllib.loads(source) != original:
            raise SystemExit("unexpected manifest change")
        manifest.write_text(source, encoding="utf-8")
    elif original["package"]["publish"] is not False:
        raise SystemExit("application publish setting requires review")
    # A private path dependency could otherwise be silently excluded.
    for path in (checkout / "src-tauri/vendor").rglob("Cargo.toml"):
        pkg = tomllib.loads(path.read_text(encoding="utf-8")).get("package", {})
        if pkg.get("publish") is False or pkg.get("publish") == []:
            raise SystemExit("private vendor requires separate notice review: " + str(path))
    print("Only the scratch application manifest is marked non-publishable; no license assigned.")

if __name__ == "__main__":
    main()
