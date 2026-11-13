#!/usr/bin/env python3
"""Generate packaging/flatpak/cargo-sources.json from Cargo.lock.

Usage: gen_flatpak_cargo_sources.py [Cargo.lock] [out.json]

Emits the same shape as flatpak-builder-tools' flatpak-cargo-generator.py for
crates.io-only lockfiles: per crate an archive source plus an inline
.cargo-checksum.json, then a cargo config that redirects crates.io to the
vendored directory. Uses only the standard library and reads Cargo.lock line by
line (its [[package]] blocks hold one `key = "value"` per line), so it runs on
any Python 3. Git dependencies are rejected: use the upstream generator then.
"""
import json
import sys
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def read_packages(path: Path):
    package = None
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("["):
            if package is not None:
                yield package
            package = {} if line == "[[package]]" else None
            continue
        match = re.match(r'^(name|version|source|checksum) = "(.*)"$', line)
        if package is not None and match:
            package[match.group(1)] = match.group(2)
    if package is not None:
        yield package


def main() -> int:
    lock_path = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "Cargo.lock"
    out_path = (
        Path(sys.argv[2])
        if len(sys.argv) > 2
        else ROOT / "packaging" / "flatpak" / "cargo-sources.json"
    )
    sources = []
    for package in read_packages(lock_path):
        source = package.get("source")
        if source is None:
            continue
        if source != REGISTRY:
            print(f"unsupported source for {package['name']}: {source}", file=sys.stderr)
            return 1
        name, version, checksum = package["name"], package["version"], package["checksum"]
        dest = f"cargo/vendor/{name}-{version}"
        sources.append(
            {
                "type": "archive",
                "archive-type": "tar-gzip",
                "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                "sha256": checksum,
                "dest": dest,
            }
        )
        sources.append(
            {
                "type": "inline",
                "contents": json.dumps({"package": checksum, "files": {}}),
                "dest": dest,
                "dest-filename": ".cargo-checksum.json",
            }
        )

    sources.append(
        {
            "type": "inline",
            "contents": '[source.vendored-sources]\ndirectory = "cargo/vendor"\n\n'
            '[source.crates-io]\nreplace-with = "vendored-sources"\n',
            "dest": "cargo",
            "dest-filename": "config",
        }
    )

    out_path.write_text(json.dumps(sources, indent=4), encoding="utf-8")
    print(f"wrote {len(sources) // 2} crates to {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
