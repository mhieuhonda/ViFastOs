---
name: vifast-parallel-gate
description: >-
  Chạy cổng kiểm thử ViFastOs song song trên 2 máy (Mac local + host Linux
  `ps` qua SSH) để rút ngắn vòng build/test; kèm các luật chống flake và cách
  đọc kết quả.
---

# Cổng song song 2 máy cho ViFastOs

## Khi nào dùng

- Cần chạy **cổng đầy đủ** (QEMU 3 arch + persist + harness + docs + fmt) mà
  không muốn chờ tuần tự.
- Máy local (Mac, arm64) chạy **riscv64 + aarch64**; host `ps` (Ubuntu x86_64)
  chạy **x86_64** — đúng arch máy, QEMU nhanh nhất.

## Chuẩn bị một lần

```sh
ssh ps 'rustup target list --installed'
# cần: riscv64gc-unknown-none-elf, riscv64imac-unknown-none-elf,
#       aarch64-unknown-none-softfloat, x86_64-unknown-none, x86_64-unknown-uefi
ssh ps 'dpkg -l | grep -E "qemu-system-(x86|arm|riscv)|ovmf"'
# symlink OVMF nếu cần:
#   /usr/share/OVMF/OVMF_CODE.fd -> OVMF_CODE_4M.fd
#   /usr/share/OVMF/OVMF_VARS.fd -> OVMF_VARS_4M.fd
```

`scripts/par-gate.sh` tự `rsync` repo sang `~/ViFastOs` (loại `target/`,
`content/`) — không cần copy tay.

## Cách chạy

```sh
./scripts/par-gate.sh --repeat 1 --full        # cổng đầy đủ, 2 máy
./scripts/par-gate.sh --repeat 1               # chỉ QEMU 3 arch
./scripts/par-gate.sh --local-only --full      # 1 máy (khi ps bận)
```

Kết quả: `par-gate: PASS` / `FAIL (local=<rc> remote=<rc>)`; log local ở
`$TMPDIR/vifast-par-local.log`, remote ở `$TMPDIR/vifast-par-remote.log`.

## Luật chống flake (bắt buộc)

1. **Không chạy 2 harness cùng arch trên một máy.** X86_64 chỉ chạy ở `ps`,
   riscv64/aarch64 chỉ chạy ở local.
2. **Serialize persist x86_64**: persist local và remote chạy đồng thời từng
   làm boot UEFI timeout không in marker nào. Script đã xếp persist/harness/
   docs **sau** khi remote xong; đừng đổi thứ tự này.
3. Khi thấy `FAIL: timeout (thiếu N marker)`: đọc danh sách marker thiếu —
   thiếu 0 marker = flake do tải máy; thiếu marker cụ thể = lỗi thật.
4. `VIFAST_RETRY_TIMEOUT=1` (par-gate bật mặc định) chỉ retry **timeout**;
   lỗi marker/exit code vẫn fail ngay.

## Đo thời gian (2026-11, Mac arm64 + ps x86_64)

Đo trên MacBook arm64 + `ps` (Ubuntu x86_64), `--repeat 1 --full`:

| Chế độ | Lệnh | Thời gian |
|---|---|---|
| Tuần tự | `test-qemu.py --repeat 1` (3 arch) + `test-persist.py` | **107 s** |
| Song song | `par-gate.sh --repeat 1 --full` | **129 s** |

Kết luận trung thực: ở `--repeat 1`, chế độ song song **chậm hơn ~20%** vì chi
phí cố định (rsync + `build.sh` đầy đủ trên `ps` ≈ 50-60 s) lớn hơn phần tiết
kiệm từ việc đẩy x86_64 sang máy khác. Chỉ nên dùng khi:

- `--repeat >= 3` (phần QEMU tăng gấp N, chi phí cố định chia đều), hoặc
- máy local đang bận (build/agent khác) — `ps` gánh x86_64 + persist, hoặc
- cần chạy persist 2 nơi để so sánh (nhưng phải serialize, xem luật 2).

Đừng kỳ vọng song song nhanh hơn ở repeat nhỏ — đo lại trước khi tin.

## Kiểm chứng nhanh sau khi sửa script

```sh
sh -n scripts/par-gate.sh
./scripts/par-gate.sh --repeat 1 --local-only   # phải PASS trước khi bật remote
```

## Kiểm chứng 2026-11-11 (đêm r294)

**Cách vào máy `ps`:** phải chỉ định user, KHÔNG dùng user mặc định (tailnet
chặn `mps`):

```sh
ssh -l ps ps 'uname -a'        # OK: Linux ps ... x86_64, 12 CPU
ssh ps 'uname -a'              # HỎNG: "tailnet policy does not permit ... user mps"
```

`scripts/par-gate.sh` mặc định `REMOTE=ps@100.113.214.23` nên chạy được ngay;
nếu đổi tay thì dùng `--remote ps@ps`.

**Môi trường trên `ps` đã kiểm tra đủ (không cần cài thêm):**

| Thứ cần | Trạng thái |
|---|---|
| rustup target | `riscv64gc/riscv64imac-unknown-none-elf`, `aarch64-unknown-none(-softfloat)`, `x86_64-unknown-none`, `x86_64-unknown-uefi` — đủ 5 |
| QEMU | `/usr/bin/qemu-system-{x86_64,riscv64,aarch64}` |
| OVMF | `OVMF_CODE.fd → OVMF_CODE_4M.fd`, `OVMF_VARS.fd → OVMF_VARS_4M.fd` (symlink đã có) |
| Khác | `rsync`, `python3`, repo `~/ViFastOs` |
| Máy | 12 CPU, 14 GB (chạy 1 phiến QEMU thoải mái) |

**Kết quả chạy thật:** `./scripts/par-gate.sh --repeat 1` ⇒ `par-gate: PASS`,
`RETRY 0/1`, `x86_64 1/1 PASS` (x86_64 chạy ở `ps`, riscv64+aarch64 chạy local).

## Dùng để NHÂN ĐÔI thông lượng soak (cách đúng)

Chia theo **kiến trúc**, không chia theo lần lặp cùng arch (luật 1):

```sh
# Máy local: riscv64+T33b, máy ps: x86_64+T33a — chạy ĐỒNG THỜI
python3 -u scripts/test-qemu.py --arch riscv64 --repeat 100 > /tmp/riscv-100.log 2>&1 &
ssh -l ps ps 'cd ~/ViFastOs && ~/.cargo/bin/cargo build --release ... && \
  python3 -u scripts/test-qemu.py --arch x86_64 --repeat 100' > /tmp/x86-100.log 2>&1
wait
```

Thời gian ≈ `max(t_local, t_remote)` thay vì tổng. Điều kiện: **rsync mã nguồn
sang `ps` TRƯỚC** (`./scripts/par-gate.sh` làm việc này) và **hai bên dùng arch
khác nhau**. Sau khi sửa mã, nhớ rsync lại; nếu không, `ps` chạy bản cũ và kết
quả vô nghĩa — luôn đối chiếu `source_sha256` trong `target/vifast-qemu-last.json`
của hai máy.
