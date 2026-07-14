#!/usr/bin/env python3
"""Collect license files for registry crates linked into the xubamp binary."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
from collections import deque
from pathlib import Path


LICENSE_PREFIXES = ("copying", "copyright", "license", "notice")


def metadata(target: str) -> dict:
    command = [
        "cargo",
        "metadata",
        "--locked",
        "--format-version",
        "1",
        "--filter-platform",
        target,
        "--features",
        "audio,keyboard",
    ]
    return json.loads(subprocess.check_output(command, text=True))


def runtime_registry_packages(data: dict) -> list[dict]:
    packages = {package["id"]: package for package in data["packages"]}
    nodes = {node["id"]: node for node in data["resolve"]["nodes"]}
    roots = [
        package["id"]
        for package in data["packages"]
        if package["name"] == "xubamp" and package["source"] is None
    ]
    if len(roots) != 1:
        raise RuntimeError("cargo metadata did not contain one local xubamp package")

    seen: set[str] = set()
    pending = deque(roots)
    while pending:
        package_id = pending.popleft()
        if package_id in seen:
            continue
        seen.add(package_id)
        for dependency in nodes[package_id]["deps"]:
            if any(kind["kind"] is None for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])

    return sorted(
        (
            packages[package_id]
            for package_id in seen
            if packages[package_id]["source"] is not None
        ),
        key=lambda package: (package["name"], package["version"]),
    )


def license_files(package_dir: Path) -> list[Path]:
    files = [
        path
        for path in package_dir.iterdir()
        if path.is_file() and path.name.casefold().startswith(LICENSE_PREFIXES)
    ]
    licenses_dir = package_dir / "LICENSES"
    if licenses_dir.is_dir():
        files.extend(path for path in licenses_dir.rglob("*") if path.is_file())
    return sorted(files, key=lambda path: str(path.relative_to(package_dir)))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True, help="Rust host target triple")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--lockfile", default="Cargo.lock", type=Path)
    parser.add_argument("--fallback-dir", required=True, type=Path)
    parser.add_argument(
        "--common-licenses", default="/usr/share/common-licenses", type=Path
    )
    args = parser.parse_args()

    destination = args.output
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True)

    rows = []
    for package in runtime_registry_packages(metadata(args.target)):
        package_dir = Path(package["manifest_path"]).parent
        package_destination = destination / f'{package["name"]}-{package["version"]}'
        copied = []
        sources = license_files(package_dir)
        fallback = args.fallback_dir / f'{package["name"]}-{package["version"]}'
        if not sources and fallback.is_dir():
            sources = sorted(path for path in fallback.rglob("*") if path.is_file())
            package_dir = fallback
        if not sources:
            common_license = args.common_licenses / (package.get("license") or "")
            if common_license.is_file():
                sources = [common_license]
                package_dir = args.common_licenses
        if not sources:
            raise RuntimeError(
                f'{package["name"]} {package["version"]} has no packaged license text'
            )
        for source in sources:
            relative = source.relative_to(package_dir)
            target = package_destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
            copied.append(str(relative))
        rows.append(
            (
                package["name"],
                package["version"],
                package.get("license") or "not declared",
                package["source"],
                ",".join(copied),
            )
        )

    lock_hash = hashlib.sha256(args.lockfile.read_bytes()).hexdigest()
    manifest = destination / "MANIFEST.tsv"
    with manifest.open("w", encoding="utf-8", newline="\n") as output:
        output.write(f"# Cargo.lock SHA-256: {lock_hash}\n")
        output.write("name\tversion\tdeclared license\tsource\tfiles copied\n")
        for row in rows:
            output.write("\t".join(row) + "\n")


if __name__ == "__main__":
    main()
