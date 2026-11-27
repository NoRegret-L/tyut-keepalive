"""Validate the generated draft release manifest; runtime updater verifies cryptography."""
import base64
import json
from pathlib import Path
import sys
from urllib.parse import urlsplit, unquote

def require(condition, message):
    if not condition:
        raise SystemExit("Updater manifest check failed: " + message)

def check(data, tag, assets=None):
    version = tag.removeprefix("v")
    require(data.get("version", "").removeprefix("v") == version, "manifest version differs from tag")
    platforms = data["platforms"]
    asset_sizes = {a["name"]: a["size"] for a in assets} if assets is not None else None
    for target, bundle, extensions in [
        ("windows-x86_64", "msi", (".msi", ".msi.zip")),
        ("darwin-aarch64", "app", (".app.tar.gz",)),
        ("darwin-x86_64", "app", (".app.tar.gz",)),
        ("linux-x86_64", "appimage", (".appimage", ".appimage.tar.gz")),
    ]:
        candidates = [platforms[k] for k in (target, target + "-" + bundle) if k in platforms]
        require(bool(candidates), "missing " + target)
        for item in candidates:
            url = urlsplit(item["url"])
            prefix = "/NoRegret-L/tyut-keepalive/releases/download/" + tag + "/"
            require(url.scheme == "https" and url.netloc == "github.com" and url.path.startswith(prefix)
                    and not url.query and not url.fragment, "untrusted asset URL")
            name = unquote(url.path[len(prefix):])
            require(name and "/" not in name and "\\" not in name and name.lower().endswith(extensions), "incorrect target asset")
            if asset_sizes is not None:
                require(asset_sizes.get(name, 0) > 0 and asset_sizes.get(name + ".sig", 0) > 0, "missing package or signature asset")
            signature = base64.b64decode(item["signature"], validate=True).decode("utf-8")
            comments = [line for line in signature.splitlines() if line.startswith("trusted comment:")]
            require(len(comments) == 1 and ("version:" + version) in comments[0].split("\t"), "missing matching signed version")
    print("Updater manifest covers Windows, Linux and both macOS architectures and declares matching signed versions")

if __name__ == "__main__":
    assets = json.loads(Path(sys.argv[3]).read_text(encoding="utf-8")) if len(sys.argv) > 3 else None
    check(json.loads(Path(sys.argv[1]).read_text(encoding="utf-8")), sys.argv[2], assets)
