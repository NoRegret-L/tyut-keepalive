"""Collect only expected installer files; hash each and record exact commit."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]
def require(condition, message):
    if not condition:
        raise SystemExit("Test artifact check failed: " + message)

def verify_deb_members(archive):
    """Inspect exact archive paths, not locale-dependent dpkg -c text."""
    executable = "usr/bin/tyut-keepalive-test"
    production = "usr/bin/tyut-keepalive"
    matches = []
    production_paths = []
    for member in archive:
        path = PurePosixPath(member.name).as_posix()
        if path.startswith("usr/bin/") or path.startswith("./usr/bin/"):
            print(f"DEB executable entry: {path} (mode {member.mode:o})")
        if path == executable:
            matches.append(member.isfile() and bool(member.mode & 0o111))
        if path == production:
            production_paths.append(path)
    require(not production_paths, "DEB contains production executable path")
    require(matches == [True], "DEB must contain exactly one regular executable at usr/bin/tyut-keepalive-test")

def verify_deb(package):
    name = subprocess.check_output(["dpkg-deb", "-f", str(package), "Package"], text=True).strip()
    require(name == "tyut-keepalive-test", "unexpected Debian package name: " + name)
    # Stream the filesystem archive without extracting files or executing the package.
    with subprocess.Popen(["dpkg-deb", "--fsys-tarfile", str(package)], stdout=subprocess.PIPE) as proc:
        with tarfile.open(fileobj=proc.stdout, mode="r|") as archive:
            verify_deb_members(archive)
        proc.stdout.read()
        require(proc.wait() == 0, "dpkg-deb filesystem archive read failed")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--platform", choices=["windows", "linux", "macos"], required=True)
    parser.add_argument("--target", required=True)
    args = parser.parse_args()
    expected = {"windows": "x86_64-pc-windows-msvc", "linux": "x86_64-unknown-linux-gnu", "macos": "universal-apple-darwin"}
    require(args.target == expected[args.platform], "wrong architecture")
    bundle = ROOT / "src-tauri/target" / args.target / "release/bundle"
    patterns = {"windows": [("msi", "*.msi")], "linux": [("deb", "*.deb"), ("appimage", "*.AppImage")], "macos": [("dmg", "*.dmg")]}[args.platform]
    if args.platform == "macos":
        import plistlib
        apps = list((bundle / "macos").glob("*.app"))
        require(len(apps) == 1, "expected one macOS app")
        with (apps[0] / "Contents/Info.plist").open("rb") as file:
            info = plistlib.load(file)
        require(info["CFBundleIdentifier"] == "com.tyut.keepalive.test", "wrong macOS bundle identity")
        executable = apps[0] / "Contents/MacOS" / info["CFBundleExecutable"]
        require(executable.name == "tyut-keepalive-test", "wrong macOS executable")
        archs = subprocess.check_output(["lipo", "-archs", str(executable)], text=True).split()
        require(set(archs) == {"arm64", "x86_64"}, "not a universal macOS binary")
    out = ROOT / "test-artifacts"
    out.mkdir(exist_ok=False)
    files = []
    for directory, pattern in patterns:
        candidates = sorted((bundle / directory).glob(pattern))
        require(len(candidates) == 1, "expected exactly one " + pattern)
        package = candidates[0]
        require(package.is_file() and package.stat().st_size > 0, "empty package")
        require("test" in package.name.lower(), "installer filename lacks test marker")
        if directory == "deb":
            verify_deb(package)
        if directory == "dmg":
            subprocess.run(["hdiutil", "verify", str(package)], check=True)
        target = out / package.name
        shutil.copy2(package, target)
        files.append(target)
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    require(commit == os.environ.get("GITHUB_SHA", commit), "checkout mismatch")
    metadata = {"channel": "test-only", "commit": commit, "platform": args.platform,
                "target": args.target, "version": json.loads((ROOT / "src-tauri/tauri.test.conf.json").read_text(encoding="utf-8"))["version"], "identifier": "com.tyut.keepalive.test",
                "updater": "disabled", "systemCodeSigning": "not configured"}
    (out / "BUILD-INFO.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    files.append(out / "BUILD-INFO.json")
    with (out / "SHA256SUMS.txt").open("w", encoding="utf-8", newline="\n") as checks:
        for path in sorted(files):
            with path.open("rb") as file:
                digest = hashlib.file_digest(file, "sha256").hexdigest()
            checks.write(f"{digest}  {path.name}\n")
            print(f"{digest}  {path.name}")

if __name__ == "__main__":
    main()
