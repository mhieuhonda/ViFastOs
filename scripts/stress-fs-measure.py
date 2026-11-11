#!/usr/bin/env python3
"""Đo T26 bằng vòng lặp FS trong một lần boot (feature `stress-fs`, r295).

Vì sao cần script này: lỗi T26 chỉ lộ ra qua nhiều lần BOOT, nhưng mỗi lần boot
cũ chỉ có MỘT phép thử (`mkdir → readdir` trong khối V2). Với `stress-fs`,
một lần boot cho `K` phép thử, nên 5 boot là đủ để kết luận — và mẫu số phải
được ghi bằng SỐ VÒNG, không phải số boot.

Script KHÔNG rebuild (bài học r280): chạy
    VIFAST_CLIENT_FEATURES=stress-fs ./scripts/build.sh
trước, rồi
    python3 scripts/stress-fs-measure.py --label S1 --iterations 5

Mỗi lần boot được chạy qua `scripts/test-qemu.py --arch x86_64` với
`VIFAST_NO_RETRY=1` (đúng MỘT boot cho mỗi lần gọi) và `VIFAST_KEEP_SERIAL=1`.
Serial đầy đủ được lưu vào `artifacts/r295-<label>/`.

Đọc số (đúng luật của tệp quyết định §6): mỗi dòng kết quả ghi kèm id BUILD,
máy, arch, số vòng và số boot.
"""

from __future__ import annotations

import argparse
import glob
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ABI = os.path.join(ROOT, "crates", "vifast-abi", "src", "lib.rs")

LOOP_RE = re.compile(
    r"FS LOOP k=(\d+) miss_rd=(\d+) miss_open=(\d+) err=(\d+)"
)
VSTAT_RE = re.compile(
    r"VSTAT vseq=(\d+) varg=(\d+) vhash=(\d+) vend=(\d+)"
)
BUILD_RE = re.compile(r"^BUILD (\w+)=(\S+)", re.MULTILINE)


def read_max_io() -> int:
    source = open(ABI, encoding="utf-8").read()
    match = re.search(r"const MAX_IO:\s*usize\s*=\s*(\d+)", source)
    if match is None:
        raise SystemExit("không tìm thấy MAX_IO trong vifast-abi/src/lib.rs")
    return int(match.group(1))


def run_boot(label: str, index: int) -> tuple[dict, str]:
    """Boot một lần; trả (số đo, đường dẫn serial đã lưu)."""
    env = dict(os.environ)
    env["VIFAST_NO_RETRY"] = "1"
    env["VIFAST_KEEP_SERIAL"] = "1"
    env.setdefault("VIFAST_TIMEOUT", "400")
    started = time.time()
    proc = subprocess.run(
        [sys.executable, os.path.join(ROOT, "scripts", "test-qemu.py"), "--arch", "x86_64"],
        cwd=ROOT,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    elapsed = time.time() - started
    console = proc.stdout.decode("utf-8", errors="replace")
    kept = sorted(
        glob.glob(os.path.join(tempfile.gettempdir(), "vifast-keep-x86_64-*.serial")),
        key=os.path.getmtime,
    )
    # Lọc theo thời điểm: file vừa được ghi trong lần chạy này.
    fresh = [path for path in kept if os.path.getmtime(path) >= started - 1]
    serial_text = ""
    saved = ""
    if fresh:
        saved = os.path.join(ROOT, "artifacts", f"r295-{label}", f"boot{index}.serial")
        os.makedirs(os.path.dirname(saved), exist_ok=True)
        shutil.copyfile(fresh[-1], saved)
        serial_text = open(fresh[-1], "rb").read().decode("utf-8", errors="replace")

    loop = LOOP_RE.search(serial_text)
    vstat = VSTAT_RE.search(serial_text)
    builds = sorted({match.group(2) for match in BUILD_RE.finditer(serial_text)})
    record = {
        "boot": index,
        "seconds": round(elapsed, 1),
        "build": builds[0] if len(builds) == 1 else ("LỆCH:" + ",".join(builds)),
        "v2": "OK" if "FS V2 OK" in serial_text else ("BAD" if "FS V2 BAD" in serial_text else "?"),
        "loops": int(loop.group(1)) if loop else 0,
        "miss_rd": int(loop.group(2)) if loop else -1,
        "miss_open": int(loop.group(3)) if loop else -1,
        "err": int(loop.group(4)) if loop else -1,
        "vseq": int(vstat.group(1)) if vstat else -1,
        "varg": int(vstat.group(2)) if vstat else -1,
        "vhash": int(vstat.group(3)) if vstat else -1,
        "vend": int(vstat.group(4)) if vstat else -1,
        "vring": serial_text.count("VRING srv="),
        "first": "VFIRST" in serial_text,
        "pass": " PASS" in console.split("=== summary ===")[-1],
    }
    return record, saved


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--label", default="S1", help="nhãn lượt đo (S1..S4, S1b)")
    parser.add_argument("--iterations", type=int, default=5, help="số lần BOOT")
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations phải >= 1")

    max_io = read_max_io()
    smp = os.environ.get("VIFAST_SMP", "2")
    accel = os.environ.get("VIFAST_ACCEL", "") or "mặc định"
    retry = os.environ.get("VIFAST_RETRY_TIMEOUT", "0")
    print(
        f"lượt {args.label}: MAX_IO={max_io} smp={smp} accel={accel} "
        f"arch=x86_64 máy={os.uname().nodename} boot={args.iterations} "
        f"RETRY_TIMEOUT={retry}"
    )

    records = []
    for index in range(1, args.iterations + 1):
        record, saved = run_boot(args.label, index)
        records.append(record)
        print(
            f"  boot {index}: {record['seconds']}s build={record['build']} "
            f"v2={record['v2']} loop={record['loops']} "
            f"miss_rd={record['miss_rd']} miss_open={record['miss_open']} "
            f"err={record['err']} vseq={record['vseq']} varg={record['varg']} "
            f"vhash={record['vhash']} vend={record['vend']} "
            f"vring={record['vring']} first={record['first']}"
            + (f" serial={os.path.relpath(saved, ROOT)}" if saved else " SERIAL=KHÔNG CÓ")
        )

    total_loops = sum(record["loops"] for record in records)
    total_rd = sum(max(record["miss_rd"], 0) for record in records)
    total_open = sum(max(record["miss_open"], 0) for record in records)
    total_err = sum(max(record["err"], 0) for record in records)
    bad_lights = sum(
        max(record["vseq"], 0) + max(record["varg"], 0) + max(record["vhash"], 0)
        for record in records
    )
    builds = sorted({record["build"] for record in records})
    print("--- tổng ---")
    print(f"build={','.join(builds)} MAX_IO={max_io} smp={smp} arch=x86_64")
    print(f"boot={len(records)} vòng={total_loops} miss_rd={total_rd} miss_open={total_open} err={total_err}")
    print(f"đèn BAD (vseq+varg+vhash)={bad_lights} vring_in={sum(r['vring'] for r in records)}")
    if total_loops:
        print(f"tỉ lệ miss_rd = {total_rd}/{total_loops}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
