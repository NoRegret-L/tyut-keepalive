"""Fail closed on accidental WebView privilege or updater policy expansion."""
import argparse
import base64
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parent.parent
CONFIG = ROOT / "src-tauri/tauri.conf.json"
ENDPOINT = "https://github.com/NoRegret-L/tyut-keepalive/releases/latest/download/latest.json"
COMMANDS = ["get_status","get_telemetry","set_notifications","reconnect","set_paused","reset_credentials","setup_credentials","set_autostart","is_autostart","quit","hide_window","update_enabled","check_update","install_update"]
PERMISSIONS = {"core:event:allow-listen", "core:event:allow-unlisten", "core:app:allow-version",
               *("allow-" + c.replace("_", "-") for c in COMMANDS)}

def require(condition, message):
    if not condition:
        raise SystemExit("Security check failed: " + message)

class LocalHTML(HTMLParser):
    def handle_starttag(self, tag, attrs):
        values = dict(attrs)
        require(not any(name.startswith("on") or name == "style" for name, _ in attrs), "inline executable/style attribute")
        require(tag not in {"iframe", "object", "embed", "base"}, "unexpected embedded content")
        if tag == "script":
            require(values.get("src") == "./main.js", "unexpected or inline script")
        for name in ("src", "href", "action", "formaction"):
            value = values.get(name, "")
            require(not re.match(r"^(?:[a-zA-Z][a-zA-Z0-9+.-]*:|//)", value.strip()), "external resource or form action")
    handle_startendtag = handle_starttag

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--enable-bundling", action="store_true")
    args = parser.parse_args()
    cfg = json.loads(CONFIG.read_text(encoding="utf-8"))
    security = cfg["app"]["security"]
    require(security.get("capabilities") == ["default"], "unexpected capabilities")
    csp = {p.strip().split()[0]: p.strip().split()[1:] for p in security["csp"].split(";") if p.strip()}
    for directive, values in {
        "default-src": ["'self'"], "script-src": ["'self'"], "style-src": ["'self'"],
        "connect-src": ["'self'", "ipc:", "http://ipc.localhost"],
        "img-src": ["'self'", "data:"], "object-src": ["'none'"],
        "base-uri": ["'none'"], "form-action": ["'none'"],
        "frame-src": ["'none'"], "worker-src": ["'none'"], "media-src": ["'none'"],
    }.items():
        require(csp.get(directive) == values, "unexpected CSP: " + directive)
    cap = json.loads((ROOT / "src-tauri/capabilities/default.json").read_text(encoding="utf-8"))
    require(cap.get("windows") == ["main"] and "remote" not in cap and cap.get("local", True), "unexpected capability scope")
    require(set(cap["permissions"]) == PERMISSIONS and len(cap["permissions"]) == len(PERMISSIONS), "unexpected IPC permissions")
    source = (ROOT / "dist/main.js").read_text(encoding="utf-8")
    calls = set(re.findall(r'invoke\("([a-z_]+)"', source))
    require(calls == set(COMMANDS), "frontend command allowlist mismatch")
    handler = (ROOT / "src-tauri/src/lib.rs").read_text(encoding="utf-8")
    block = handler.split("tauri::generate_handler![", 1)[1].split("]", 1)[0]
    require(set(re.findall(r"(?:commands|updates)::([a-z_]+)", block)) == set(COMMANDS), "registered command allowlist mismatch")
    manifest = (ROOT / "src-tauri/build.rs").read_text(encoding="utf-8")
    require(set(re.findall(r'"([a-z_]+)"', manifest)) == set(COMMANDS), "build permission manifest mismatch")
    LocalHTML().feed((ROOT / "dist/index.html").read_text(encoding="utf-8"))
    updater = cfg["plugins"]["updater"]
    require(updater["endpoints"] == [ENDPOINT], "unexpected update endpoint")
    require(updater["requireSignedVersion"] is True and updater["allowDowngrades"] is False
            and updater["dangerousInsecureTransportProtocol"] is False, "unsafe updater settings")
    require(cfg["bundle"]["createUpdaterArtifacts"] is False, "default build must not require a signing key")
    if args.release or args.enable_bundling:
        try:
            key = base64.b64decode(updater["pubkey"], validate=True).decode("ascii")
            require(key.startswith("untrusted comment:") and len(key.splitlines()) >= 2, "invalid updater public key")
            require(len(base64.b64decode(key.splitlines()[1], validate=True)) == 42, "invalid Minisign public key length")
        except (ValueError, TypeError, UnicodeError):
            raise SystemExit("Security check failed: configure a valid Tauri public key before release")
        version = cfg["version"]
        require(re.fullmatch(r"\d+\.\d+\.\d+", version), "release must use a stable semantic version")
        package = json.loads((ROOT / "package.json").read_text(encoding="utf-8"))
        package_lock = json.loads((ROOT / "package-lock.json").read_text(encoding="utf-8"))
        cargo = tomllib.loads((ROOT / "src-tauri/Cargo.toml").read_text(encoding="utf-8"))
        cargo_lock = tomllib.loads((ROOT / "src-tauri/Cargo.lock").read_text(encoding="utf-8"))
        root_packages = [p for p in cargo_lock["package"] if p["name"] == cargo["package"]["name"] and "source" not in p]
        require(len(root_packages) == 1, "missing app Cargo.lock entry")
        require(package["version"] == package_lock["version"] == package_lock["packages"][""]["version"]
                == cargo["package"]["version"] == root_packages[0]["version"] == version, "version mismatch")
        require(os.environ.get("GITHUB_REF_NAME") == "v" + version, "release tag/version mismatch")
        require(os.environ.get("TAURI_SIGNING_PRIVATE_KEY", "").strip(), "missing signing secret")
    if args.enable_bundling:
        cfg["bundle"]["createUpdaterArtifacts"] = True
        CONFIG.write_text(json.dumps(cfg, indent=2) + "\n", encoding="utf-8")
    print("Security configuration OK")

if __name__ == "__main__":
    main()
