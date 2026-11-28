# SPDX-License-Identifier: MPL-2.0
"""Stage auditable license resources and exact MPL source archives, never publish."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"

def require(condition, message):
    if not condition:
        raise SystemExit("License staging failed: " + message)

def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

def json_read(path):
    return json.loads(path.read_text(encoding="utf-8"))

def json_write(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

def is_notice(name):
    return name.upper().startswith(("LICENSE", "LICENCE", "COPYRIGHT", "NOTICE", "COPYING"))

def policy_entries(policy, approval):
    records = [approval["application"], *approval["packages"]]
    expected = {f'{p["name"]}@={p["version"]}': [p["license"]] for p in records}
    exceptions = policy["licenses"]["exceptions"]
    actual = {p["crate"]: p["allow"] for p in exceptions}
    require(len(records) == len(expected) == len(exceptions) == len(actual)
            and actual == expected, "package-specific policy and approval record differ")
    require(all(set(p) == {"crate", "allow"} for p in exceptions), "unexpected exception keys")
    require(not {"MPL-2.0", "Unicode-3.0", "Apache-2.0 WITH LLVM-exception"}
            .intersection(policy["licenses"]["allow"]), "reviewed terms must not be globally allowed")
    return records

def report_sections(report):
    """Use cargo-about's documented JSON schema, not delimiters in legal text."""
    licenses = report.get("licenses")
    require(isinstance(licenses, list) and licenses, "empty cargo-about report")
    result = {}
    for entry in licenses:
        ident, name, body = entry["id"], entry["name"], entry["text"]
        users = entry["used_by"]
        require(all(isinstance(x, str) and x.strip() for x in (ident, name, body))
                and isinstance(users, list) and users, "invalid cargo-about license record")
        for user in users:
            crate = user["crate"]
            key = f'{crate["name"]} {crate["version"]}'
            result.setdefault(key, []).append((ident, body))
    return result

def render_report(report):
    lines = ["THIRD-PARTY RUST LICENSE TEXTS — REVIEW CANDIDATE",
             "Not a distribution approval or an exhaustive installer notice.",
             "Includes build and development dependencies across the four supported targets.",
             "Application license, npm packages, native libraries and bundled assets require separate review.", ""]
    for entry in report["licenses"]:
        lines += ["=" * 70, f'{entry["name"]} ({entry["id"]})', "Used by:"]
        lines += [f'- {u["crate"]["name"]} {u["crate"]["version"]}' for u in entry["used_by"]]
        lines += ["", entry["text"], ""]
    return "\n".join(lines) + "\n"

def check_report(sections, package):
    key = f'{package["name"]} {package["version"]}'
    texts = sections.get(key, [])
    license_id = package["license"]
    if license_id == "Apache-2.0 WITH LLVM-exception":
        require(any(ident in ("Apache-2.0", license_id) and "LLVM" in body and "exception" in body.lower()
                    for ident, body in texts), "missing LLVM exception text: " + key)
    else:
        require(any(ident == license_id for ident, _ in texts), "missing license text: " + key)
    if package["name"] == "unicode-ident":
        require(any(ident in ("MIT", "Apache-2.0") for ident, _ in texts),
                "unicode-ident AND expression not fully represented")

def copy_crate_notices(archive, package, output):
    prefix = f'{package["name"]}-{package["version"]}'
    count = 0
    with tarfile.open(archive, "r:gz") as tar:
        for member in tar.getmembers():
            parts = Path(member.name).parts
            if len(parts) != 2 or parts[0] != prefix or not is_notice(parts[1]):
                continue
            require(member.isfile() and parts[1] not in (".", ".."), "non-regular license file")
            data = tar.extractfile(member).read()
            require(data, "empty notice " + member.name)
            dest = output / parts[1]
            require(not dest.exists(), "duplicate notice " + member.name)
            dest.write_bytes(data)
            count += 1
        if not count and (package["name"], package["version"], package["license"]) == ("selectors", "0.38.0", "MPL-2.0"):
            # This exact upstream crate declares MPL in Cargo.toml and in source
            # headers, but its published archive lacks a standalone root LICENSE.
            # The archive checksum and cargo-about coverage were verified by caller.
            members = [m for m in tar.getmembers() if m.name == prefix + "/lib.rs"]
            require(len(members) == 1 and members[0].isfile(), "missing selectors lib.rs")
            source = tar.extractfile(members[0]).read()
            header = re.match(rb"/\* This Source Code Form is subject to the terms of the Mozilla Public\r?\n \* License, v\. 2\.0\. If a copy of the MPL was not distributed with this\r?\n \* file, You can obtain one at https://mozilla\.org/MPL/2\.0/\. \*/", source)
            require(header is not None, "selectors source MPL header differs; review required")
            # Preserve the original header verbatim and explicitly label the
            # separate standard text as supplied, not an upstream original file.
            (output / "SOURCE-HEADER-lib.rs.txt").write_bytes(header.group(0) + b"\n")
            shutil.copyfile(ROOT / "LICENSE", output / "MPL-2.0-standard-supplied.txt")
            (output / "NOTICE-SOURCE.txt").write_text(
                "selectors 0.38.0: no standalone root license in published crate.\n"
                "SOURCE-HEADER-lib.rs.txt is the original source header; the standard MPL-2.0 text is supplied separately.\n"
                "The checksum-verified unmodified crate is included under sources/; all original source notices remain intact.\n",
                encoding="utf-8")
            return {"original_root_notice_files": 0, "source_header": "lib.rs",
                    "supplied_standard_text": "MPL-2.0-standard-supplied.txt"}
    require(count, "no original root license/notice file for " + prefix)
    return {"original_root_notice_files": count}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    scope = json_read(evidence / "scope.json")
    policy = tomllib.loads((ROOT / "src-tauri/deny.toml").read_text(encoding="utf-8"))
    approval = json_read(ROOT / "src-tauri/license-approvals.json")
    records = policy_entries(policy, approval)
    lock_path = ROOT / "src-tauri/Cargo.lock"
    lock = tomllib.loads(lock_path.read_text(encoding="utf-8"))["package"]
    require(digest(lock_path) == scope["cargo_lock_sha256"], "lockfile differs from graph evidence")
    require(digest(ROOT / "package-lock.json") == scope["npm_lock_sha256"], "npm lock differs")
    require(scope["application_license"] == approval["application"]["license"], "application license differs")
    require(scope["targets"] == policy["graph"]["targets"], "target set differs")
    packages = {}
    for target in scope["targets"]:
        metadata = json_read(evidence / target / "metadata.json")
        for p in metadata["packages"]:
            key = (p["name"], p["version"])
            require(key not in packages or packages[key]["id"] == p["id"], "ambiguous package " + str(key))
            packages[key] = p
    data = json_read(evidence / "cargo-about.json")
    sections = report_sections(data)
    report = evidence / "THIRD-PARTY-RUST-REVIEW.txt"
    report.write_text(render_report(data), encoding="utf-8")
    output = evidence / "bundle-resources"
    output.mkdir()
    (output / "sources").mkdir()
    (output / "original-notices").mkdir()
    shutil.copyfile(report, output / "THIRD-PARTY-LICENSES.txt")
    for filename in ("LICENSE", "LICENSING.md"):
        shutil.copyfile(ROOT / filename, output / filename)
    source_commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    require(re.fullmatch(r"[0-9a-f]{40}", source_commit), "invalid source commit")
    require(not subprocess.check_output(["git", "diff", "HEAD", "--name-only"], cwd=ROOT).strip(),
            "tracked checkout has changes; source archive would not match build")
    app_archive = output / "sources" / f"tyut-keepalive-{source_commit}.tar.gz"
    subprocess.run(["git", "archive", "--format=tar.gz", f"--output={app_archive}", source_commit],
                   cwd=ROOT, check=True)
    require(app_archive.stat().st_size > 0, "empty project source archive")
    source_rows = [{
        "name": approval["application"]["name"], "version": approval["application"]["version"],
        "license": "MPL-2.0", "archive": "sources/" + app_archive.name,
        "sha256": digest(app_archive), "url": f"https://github.com/NoRegret-L/tyut-keepalive/archive/{source_commit}.tar.gz"
    }]
    covered = []
    for package in approval["packages"]:
        key = (package["name"], package["version"])
        check_report(sections, package)
        matches = [p for p in lock if (p["name"], p["version"]) == key]
        require(len(matches) == 1 and matches[0].get("source") == REGISTRY, "not a unique registry lock entry " + str(key))
        p = packages.get(key)
        require(p and p["source"] == REGISTRY and package["license"] in p["license"], "metadata identity/license differs " + str(key))
        manifest = Path(p["manifest_path"])
        registry_dir = manifest.parent.parent
        require(registry_dir.parent.name == "src", "unexpected registry layout " + str(registry_dir))
        filename = f"{key[0]}-{key[1]}.crate"
        archive = registry_dir.parent.parent / "cache" / registry_dir.name / filename
        checksum = matches[0]["checksum"]
        require(archive.is_file() and digest(archive) == checksum, "missing or invalid original crate archive " + filename)
        notices = output / "original-notices" / f"{key[0]}-{key[1]}"
        notices.mkdir()
        notice_evidence = copy_crate_notices(archive, package, notices)
        row = {"name": key[0], "version": key[1], "license": p["license"],
               "sha256": checksum, "url": f"https://static.crates.io/crates/{key[0]}/{filename}",
               **notice_evidence}
        if package["license"] == "MPL-2.0":
            shutil.copyfile(archive, output / "sources" / filename)
            source_rows.append({**row, "archive": "sources/" + filename})
        covered.append(row)
    # Preserve original notices of the two vendored dependencies (including the GLib backport).
    for vendor in sorted((ROOT / "src-tauri/vendor").iterdir()):
        if not vendor.is_dir():
            continue
        dest = output / "original-notices" / vendor.name
        dest.mkdir()
        files = [p for p in vendor.iterdir() if p.is_file() and is_notice(p.name)]
        require(files, "missing vendor license " + vendor.name)
        for p in files:
            shutil.copyfile(p, dest / p.name)
    # NPM install in this job omits dev packages and disables lifecycle scripts.
    npm = json_read(ROOT / "package-lock.json")["packages"]
    for path, package in sorted(npm.items()):
        if not path or package.get("dev"):
            continue
        local = ROOT / path
        if package.get("optional") and not local.is_dir():
            continue
        require(local.is_dir(), "missing npm package " + path)
        name = path.removeprefix("node_modules/").replace("/", "-").replace("@", "")
        dest = output / "original-notices" / ("npm-" + name + "-" + package["version"])
        dest.mkdir()
        files = [p for p in local.iterdir() if p.is_file() and is_notice(p.name)]
        require(files, "missing npm license " + path)
        for p in files:
            shutil.copyfile(p, dest / p.name)
    json_write(output / "SOURCE-ACCESS.json", {"commit": source_commit, "sources": source_rows, "reviewed_crates": covered})
    lines = ["# Source availability and license review", "",
             "This is a CI review package, not an approval of native libraries or installer distribution.",
             f"Exact checkout: {source_commit}", "",
             "MPL-covered source archives are included locally under sources/ at no additional charge.",
             "The project archive also includes the vendored patches and build configuration.",
             "Keep this file, the original notices and archives with any distribution of these covered versions.",
             "For a future official distribution, the source archive must correspond to the actual shipped checkout.", ""]
    for row in source_rows:
        lines += [f'- {row["name"]} {row["version"]}: {row["archive"]}',
                  f'  SHA-256: {row["sha256"]}', f'  Alternate URL: {row["url"]}']
    lines += ["", "Third-party text inventory includes build/test dependencies; it is conservative, not exact binary linkage.",
              "Native libraries, bundled assets and all final installation formats remain subject to Issue #3."]
    (output / "SOURCE-ACCESS.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    sums = {p.relative_to(output).as_posix(): digest(p) for p in sorted(output.rglob("*")) if p.is_file()}
    (output / "SHA256SUMS.txt").write_text("".join(f"{h}  {p}\n" for p, h in sums.items()), encoding="utf-8")
    print(f"Staged {len(covered)} approved crates, {len(source_rows)} source archives and {len(sums)} resource files.")

if __name__ == "__main__":
    main()
