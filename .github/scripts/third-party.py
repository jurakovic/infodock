#!/usr/bin/env python3
"""Lists the crates in Infodock's Windows build with their licences (SPEC §10.7 About).

Run from the repository root after `Cargo.lock` changes:

    python3 .github/scripts/third-party.py

It asks `cargo tree` for the crates the Windows build links (normal dependencies, directly or
through others, with the features that build resolves; proc-macros run only while compiling) and
writes `assets/third-party.tsv`: one line per package in `Cargo.lock` (Infodock itself left out),
sorted by name and version, with its licence when the Windows build links it and `-` when it is
only a build, dev, proc-macro or other-platform dependency. The About page shows the first kind;
`settings::about::tests` checks that the file lists exactly the packages in `Cargo.lock`, so it
fails until this script is run again.
"""

import subprocess
import sys
from pathlib import Path

TARGET = "x86_64-pc-windows-msvc"
OUT = Path("assets/third-party.tsv")
HEADER = "# name\tversion\tlicence (- = not in the Windows build); written by .github/scripts/third-party.py\n"


def main() -> int:
    tree = subprocess.run(
        [
            "cargo", "tree", "--locked", "--target", TARGET,
            "--edges", "normal,no-proc-macro", "--prefix", "none", "--format", "{p}\t{l}",
        ],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    licences = {}
    for line in tree.splitlines():
        package, _, licence = line.partition("\t")
        # `name v1.2.3`, then ` (*)` for one listed before, or the path of a local package.
        name, version = package.split(" ")[:2]
        if name == "infodock" or not version.startswith("v"):
            continue
        licence = licence.removesuffix(" (*)").strip()
        licences[(name, version[1:])] = licence or "see its licence file"

    # Every package in the lock file, also those the Windows build does not link.
    lock = Path("Cargo.lock").read_text(encoding="utf-8")
    locked = set()
    for block in lock.split("[[package]]")[1:]:
        fields = dict(
            line.split(" = ", 1) for line in block.strip().splitlines() if " = " in line
        )
        name, version = fields["name"].strip('"'), fields["version"].strip('"')
        if name != "infodock":
            locked.add((name, version))

    missing = set(licences) - locked
    if missing:
        print(f"not in Cargo.lock: {sorted(missing)}", file=sys.stderr)
        return 1
    lines = [
        f"{name}\t{version}\t{licences.get((name, version), '-')}\n"
        for name, version in sorted(locked, key=lambda nv: (nv[0], version_key(nv[1])))
    ]
    OUT.write_text(HEADER + "".join(lines), encoding="utf-8", newline="\n")
    print(f"{OUT}: {len(licences)} crates in the Windows build, {len(locked)} packages in all")
    return 0


def version_key(version: str):
    """Sorts 0.9.0 before 0.10.0."""
    core = version.split("+")[0].split("-")[0]
    return [int(part) if part.isdigit() else 0 for part in core.split(".")], version


if __name__ == "__main__":
    sys.exit(main())
