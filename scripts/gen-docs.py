#!/usr/bin/env python3
"""Regenerate the mechanical sections of the architecture summary."""

from __future__ import annotations

import hashlib
import os
import re
import subprocess
import sys
from pathlib import Path

import harness_spec

ROOT = Path(__file__).resolve().parent.parent
SUMMARY = ROOT / "ViFastOs-architecture-summary.txt"
BUILD_LOG = ROOT / "target" / "vifast-build.log"
PACKAGES = [
    "vifast-abi",
    "vifast-kernel",
    "vifast-arch",
    "vifast-pte",
    "vifast-elf",
    "vifast-runtime",
    "vifast-user",
    "vifast-acpi",
    "vifast-fs",
    "vifast-fsutil",
]

MARKERS = {
    "commits": ("### BEGIN GENERATED commits", "### END GENERATED commits"),
    "syscalls": ("### BEGIN GENERATED syscalls", "### END GENERATED syscalls"),
    "tests": ("### BEGIN GENERATED tests", "### END GENERATED tests"),
    "pass": ("### BEGIN GENERATED pass", "### END GENERATED pass"),
    "status": ("### BEGIN GENERATED status", "### END GENERATED status"),
    "errors": ("### BEGIN GENERATED errors", "### END GENERATED errors"),
    "limits": ("### BEGIN GENERATED limits", "### END GENERATED limits"),
}


def git_commits() -> str:
    log = subprocess.check_output(
        ["git", "log", "--reverse", "--format=%h %s", "456d5ef..HEAD"],
        cwd=ROOT,
        text=True,
    ).splitlines()
    lines = ["- 456d5ef checkpoint: multi-arch microkernel before hardening"]
    lines += [
        f"- {line}"
        for line in log
        if not line.split(" ", 1)[1].startswith("docs:")
    ]
    return "\n".join(lines)


def syscalls() -> str:
    source = (ROOT / "crates" / "vifast-abi" / "src" / "lib.rs").read_text()
    match = re.search(r"pub enum SyscallId \{(.*?)\n\}", source, re.S)
    if not match:
        raise SystemExit("gen-docs: SyscallId enum not found")
    entries = re.findall(r"^\s*(\w+)\s*=\s*(\d+)\s*,", match.group(1), re.M)
    if len(entries) < 40:
        raise SystemExit("gen-docs: SyscallId parse unexpectedly short")
    status_match = re.search(r"pub const SYSCALL_STATUS:.*?= &\[(.*?)\n\];", source, re.S)
    if not status_match:
        raise SystemExit("gen-docs: SYSCALL_STATUS table not found")
    status = {
        name: value == "true"
        for name, value in re.findall(
            r"\(SyscallId::(\w+),\s*(true|false)\)", status_match.group(1)
        )
    }
    lines = []
    for name, number in entries:
        if name not in status:
            raise SystemExit(f"gen-docs: syscall {name} missing status")
        label = "implemented" if status[name] else "placeholder (NotImplemented)"
        lines.append(f"{number} {name} — {label}")
    return "\n".join(lines)


def build_results() -> str:
    if not BUILD_LOG.exists():
        raise SystemExit(
            "gen-docs: target/vifast-build.log missing; run ./scripts/build.sh first"
        )
    text = BUILD_LOG.read_text()
    counts: dict[str, tuple[int, int]] = {}
    current: str | None = None
    for line in text.splitlines():
        match = re.search(r"Running unittests .*deps/(vifast_[a-z_]+)-", line)
        if match:
            current = match.group(1).replace("_", "-")
            continue
        match = re.search(r"test result: ok\. (\d+) passed; (\d+) failed", line)
        if match and current and current not in counts:
            counts[current] = (int(match.group(1)), int(match.group(2)))
    missing = [package for package in PACKAGES if package not in counts]
    if missing:
        raise SystemExit(f"gen-docs: build log missing test results for {missing}")
    lines = []
    for package in PACKAGES:
        passed, failed = counts[package]
        lines.append(f"- `cargo test -p {package}`: {passed}/{passed + failed} PASS")
    return "\n".join(lines)


def pass_conditions() -> str:
    lines = ["Điều kiện chung (mọi kiến trúc):"]
    lines += [f"- `{value}` — {description}" for value, description in harness_spec.COMMON]
    lines.append("")
    lines.append("Mỗi dòng sau đúng một lần:")
    lines += [f"- `{value}` — exactly once" for value in harness_spec.EXACT_ONE]
    lines.append("")
    lines.append("Cấm xuất hiện:")
    lines += [f"- `{value}` — forbidden" for value in harness_spec.FORBIDDEN]
    lines.append("")
    lines.append("Exit code:")
    for arch, codes in harness_spec.EXIT_CODES.items():
        lines.append(f"- {arch}: {', '.join(str(code) for code in sorted(codes))}")
    lines.append("")
    lines.append("Theo kiến trúc:")
    for arch in ("riscv64", "aarch64", "x86_64"):
        lines.append(f"- {arch}:")
        for value, description in harness_spec.ARCH[arch]:
            lines.append(f"  - `{value}` — {description}")
        for rule in harness_spec.NUMERIC[arch]:
            lines.append(f"  - {rule}")
    lines.append("")
    lines.append("Boot stack (mọi kiến trúc):")
    lines.append("- `BOOT STACK used=U size=S` — exactly once; S >= 131072, 1 <= U*2 <= S")
    lines.append("")
    lines.append("Fault rule:")
    lines.append("- 6 + F USER FAULT records: 5 địa chỉ cũ bắt buộc + F fault trong dải pager (F <= 64)")
    lines.append("")
    lines.append("Feature test-destroy-smp:")
    lines.append("- `DESTROY SMP OK (frames=20, vspaces=2)` — exactly once")
    lines.append("- `DESTROY SMP BAD` — forbidden")
    lines.append("- build thường không được chứa `DESTROY SMP OK`")
    lines.append("")
    lines.append("Thứ tự bắt buộc:")
    for arch, pairs in harness_spec.ORDER.items():
        for before, after in pairs:
            lines.append(f"- {arch}: `{before}` trước `{after}`")
    return "\n".join(lines)


def source_fingerprint() -> str:
    """Giống hệt scripts/test-qemu.py: hash tên tương đối + nội dung, sắp xếp
    theo chuỗi tên tương đối (Path sort khác string sort nên phải thống nhất)."""
    root = ROOT
    entries = []
    for base in (os.path.join(root, "crates"), os.path.join(root, "scripts", "build.sh")):
        if os.path.isfile(base):
            entries.append(base)
            continue
        for dirpath, _dirnames, filenames in os.walk(base):
            for name in filenames:
                if name.endswith((".rs", ".S", ".toml", ".ld")) or name == "build.sh":
                    entries.append(os.path.join(dirpath, name))
    for extra in (
        os.path.join(root, "scripts", "harness_spec.py"),
        os.path.join(root, "scripts", "test-qemu.py"),
        os.path.join(root, "scripts", "gate.sh"),
        os.path.join(root, "scripts", "soak.py"),
    ):
        if os.path.isfile(extra):
            entries.append(extra)
    digest = hashlib.sha256()
    for path in sorted(entries, key=lambda item: os.path.relpath(item, root)):
        digest.update(os.path.relpath(path, root).encode())
        with open(path, "rb") as handle:
            for chunk in iter(lambda: handle.read(1 << 20), b""):
                digest.update(chunk)
    return digest.hexdigest()


def qemu_status() -> str:
    result_path = ROOT / "target" / "qemu-status-standard.json"
    if not result_path.exists():
        raise SystemExit(
            "gen-docs: target/qemu-status-standard.json missing; "
            "run test-qemu.py --repeat 3 first"
        )
    import json as _json

    try:
        payload = _json.loads(result_path.read_text())
    except (OSError, ValueError):
        payload = {}
    stored_source = payload.get("source_sha256")
    if stored_source:
        if stored_source != source_fingerprint():
            raise SystemExit(
                "gen-docs: kernel sources changed since the standard QEMU run; "
                "rerun test-qemu.py --repeat 3"
            )
    else:
        build_path = ROOT / "target" / "vifast-build.log"
        if build_path.exists() and result_path.stat().st_mtime < build_path.stat().st_mtime:
            raise SystemExit(
                "gen-docs: standard QEMU status is older than the kernel build; "
                "rerun test-qemu.py --repeat 3"
            )
    import json

    data = json.loads(result_path.read_text())
    results = data.get("results", {})
    if not results:
        raise SystemExit("gen-docs: QEMU result file has no results")
    if data.get("repeat") != 3:
        raise SystemExit(
            "gen-docs: status must come from the standard run (--repeat 3); "
            f"last run was repeat={data.get('repeat')}"
        )
    if set(results) != {"riscv64", "aarch64", "x86_64"}:
        raise SystemExit(
            "gen-docs: status needs all three architectures; "
            f"got {sorted(results)}"
        )
    pieces = [
        f"{name} {value[0]}/{value[1]}"
        for name, value in results.items()
        if isinstance(value, list) and len(value) == 2
    ]
    build = BUILD_LOG.read_text()
    match = re.search(
        r"RISC-V kernel_end=(0x[0-9a-f]+|\d+) limit=\S+ headroom=(\d+)", build
    )
    if not match:
        raise SystemExit("gen-docs: build log missing RISC-V headroom")
    end = match.group(1)
    if not end.startswith("0x"):
        end = hex(int(end))
    layout = re.search(
        r"LAYOUT kernel=(\d+) object=(\d+) tcb=(\d+) cap=(\d+)", build
    )
    if not layout:
        raise SystemExit("gen-docs: build log missing LAYOUT sizes")
    return "\n".join(
        [
            f"- RISC-V headroom: {match.group(2)} byte (`kernel_end={end}`)",
            f"- `Kernel = {layout.group(1)}`, `Object = {layout.group(2)}`, "
            f"`Tcb = {layout.group(3)}`, `Cap = {layout.group(4)}` byte",
            f"- QEMU lần chạy gần nhất (`--repeat {data.get('repeat', '?')}`): "
            + ", ".join(pieces),
        ]
    )


def error_codes() -> str:
    text = (ROOT / "crates" / "vifast-abi" / "src" / "lib.rs").read_text()
    match = re.search(r"pub enum KError\s*\{(.*?)\n\}", text, re.S)
    if not match:
        raise SystemExit("gen-docs: cannot find KError enum")
    lines = []
    for line in match.group(1).splitlines():
        item = re.match(r"\s*([A-Z][A-Za-z0-9_]*)\s*=\s*(\d+),", line)
        if item:
            lines.append(f"- `KError::{item.group(1)} = {item.group(2)}`")
    if not lines:
        raise SystemExit("gen-docs: KError enum is empty")
    return "\n".join(lines)

def limits() -> str:
    lines = []
    seen = set()
    for source, crate in (
        (ROOT / "crates" / "vifast-kernel" / "src" / "object.rs", "vifast-kernel"),
        (ROOT / "crates" / "vifast-abi" / "src" / "lib.rs", "vifast-abi"),
        (ROOT / "crates" / "vifast-fs" / "src" / "lib.rs", "vifast-fs"),
    ):
        text = source.read_text()
        for match in re.finditer(r"pub const (MAX_[A-Z0-9_]+): usize = (\d+);", text):
            name = match.group(1)
            if (crate, name) in seen:
                continue
            seen.add((crate, name))
            lines.append(f"- `{crate}::{name} = {match.group(2)}`")
    if not lines:
        raise SystemExit("gen-docs: no MAX_* limits found")
    return "\n".join(lines)

def generated() -> dict[str, str]:
    return {
        "commits": git_commits(),
        "syscalls": syscalls(),
        "tests": build_results(),
        "pass": pass_conditions(),
        "status": qemu_status(),
        "errors": error_codes(),
        "limits": limits(),
    }


def replace_sections(text: str, values: dict[str, str]) -> str:
    for name, (begin, end) in MARKERS.items():
        start = text.find(begin)
        if start < 0:
            raise SystemExit(f"gen-docs: missing marker {begin!r}")
        start = text.index("\n", start) + 1
        stop = text.find(end, start)
        if stop < 0:
            raise SystemExit(f"gen-docs: missing marker {end!r}")
        body = values[name]
        text = text[:start] + body + "\n" + text[stop:]
    return text


def render() -> str:
    return replace_sections(SUMMARY.read_text(), generated())


def main() -> int:
    values = generated()
    original = SUMMARY.read_text()
    updated = replace_sections(original, values)
    if "--check" in sys.argv:
        if updated != original:
            print("gen-docs: summary is stale", file=sys.stderr)
            return 1
        print("gen-docs: summary is current")
        return 0
    SUMMARY.write_text(updated)
    print("gen-docs: updated summary")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
