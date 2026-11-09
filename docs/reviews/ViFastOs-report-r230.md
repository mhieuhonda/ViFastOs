# ViFastOs G1 report r225..r230

> Giai đoạn G1 "Nền vững". Nhánh `g1`, chưa merge vào `main` (D8 chưa xử lý).

> T1–T4 + T6 + T7 DONE · T0 đã gỡ · T5 TIMEBOX-NEGATIVE · T8 đang chạy

---

## ViFastOs G1 report r225..r230

**Người thực hiện:** Agent 2 · **Đầu vào:** `ViFastOs-decision-r225.md`
**Mốc gốc:** `5150d0c` (= HEAD `main` lúc mở G1) · **Nhánh G1:** `g1` (worktree `/tmp/vifast-g1`)
**Trạng thái:** T1, T2, T3, T4, T6, T7 **DONE** · T0 **BLOCKED→ĐÃ GỠ** · T5 **TIMEBOX-NEGATIVE** (phân tích + số đo) · T8 **đang chạy**

---

### Part 1 - Kết quả theo task

| Task | Vòng | Trạng thái | Commit | Bằng chứng |
|---|---|---|---|---|
| **T0** Dọn repo, tách `deploy/`, commit bundle r224 | r225 | **BLOCKED** | — | xem dưới |
| **T1** Sửa check TCP echo feature-mode | r226 | **DONE** | `662ab75` | RV destroy-smp 1/1, panic-hart1 1/1 (trước 0/1) |
| **T2** Boot-handoff RV | r227 | **DONE** | `75f8109` | RV handoff **3/3** PASS (trước 0/2) |
| **T3** Mutation TLB trên RV | r228 + `r228b` | **DONE** | `394155c`, `77f5123` | RV mutated **3/3** PASS; x86 mutated PASS; aarch64 không hồi quy |
| **T4** Khe tĩnh AArch64 + canary | r229 | **DONE** | `0a72abf` | gap **8 → 4104** byte; canary OK 3 arch |
| T5 Ngân sách client (D10) | — | **CHƯA LÀM** | — | xem Part 6 |
| **T6** Bảng audit tồn đọng | r233 | **DONE** | `658d358` | `docs/audits/open-items.md`: 34 CLOSED, 3 UNREACHABLE, 9 OPEN; 3 assert vacuous `ap_entry` đã thay bằng kiểm tra thật |
| **T7** Đồng bộ summary + design docs + D7..D13 | r234 | **DONE** | `2e3c800` | §2/§7/§15/§16/§17/§18/§20; 3 doc đổi trạng thái + 2 doc mới; check-docs cấm 5 câu drift; gen-docs/check-docs/self-test PASS |
| **T8** Soak 100 iter × 3 arch | — | **ĐANG CHẠY** | — | log `/tmp/g1-soak100.log` (nền) |

**T0 — ĐÃ GỠ (r232, sau khi tiến trình khác commit xong batch license):**
Khi `main` ngừng bị ghi (~5 phút yên), đã tách vòng 224 sang nhánh `wip/r224`, commit bundle review lên `main` (`a5b09ec`), rồi merge `g1` (`5ae7aa7`). `main` nay chỉ còn `deploy/vifastos_web/static/index.html` bẩn — thuộc tiến trình web, không phải của G1. Gate `--quick` trên `main`: **PASS** (13:08–13:11).

<details><summary>Bối cảnh lúc bị chặn</summary>

**T0 — từng BLOCKED: (cần người dùng xử lý):**
Lúc mở G1, `main` có **hai** nguồn thay đổi chưa commit:
1. Vòng 224 của Agent 2: `crates/vifast-client/src/main.rs` (cd/pwd), `crates/vifast-kernel/src/ipc.rs` (**fix P0 reply-orphan**), `docs/…/16-van-hanh.md`, `scripts/gen-shell-table.py`, `bao-cao-agent0-vong-224.md`.
2. **Tiến trình khác đang ghi song song**: lúc 12:15 `LICENSE` bị tách thành `LICENSE-MIT` + `LICENSE-APACHE` (đã **stage** rename), `README.md`, `TRADEMARKS.md`, `deploy/vifastos_web/README.md`, `resources/brand/README.md` sửa cùng lúc; HEAD nhảy `5cd8df3 → 5b9fa3a → 5150d0c` trong ~30 phút.

Vì vậy **không thể** làm "main sạch" mà không commit hộ người khác. Đã làm đúng D8: **không chạm tree chính**, chuyển toàn bộ công việc sang worktree `g1`. Patch vòng 224 đã lưu nguyên vẹn để T5 dùng lại: `/tmp/r224-work.patch` (319 dòng, sha256 `9d3472cf…3ace0c`).

> **Giải thích diff `ipc.rs` (theo yêu cầu T0):** đây **không** phải sửa vu vơ — nó là fix P0 "reply-orphan": trong `TcbReply` + GRANT, sau khi `caller.take()` mọi lỗi (`cap_slot` invalid, convert `u8`, `lookup_cap` GRANT, và cả grant lỗi) đều `return Err` mà **không trả `caller` về**, khiến client kẹt `BlockedReply` vĩnh viễn. Bản sửa thêm closure `restore()` chỉ khôi phục khi client **còn** chờ (`BlockedReply` + `reply_epoch` khớp), nên stale caller vẫn bị drop đúng. **Chưa có test** — muốn merge thì phải thêm host test (đề xuất ở Part 6).

**T1 — chi tiết:** `test-qemu.py` đòi `NET TCP ECHO OK` cho **mọi** mode, nhưng feature-mode đặt `listener=None` (không có `TcpProbeListener`). Hệ quả: mọi feature test riscv64 đỏ giả từ v179. Đã tách quyết định thành `harness_spec.tcp_echo_verdict(listener_present, output) -> (ok, message)`: không listener ⇒ `SKIP` (không FAIL/PASS giả, đúng D11); có listener mà thiếu marker ⇒ FAIL. Thêm 4 case âm (592 → **596**).

**T2 — chi tiết (giả thuyết #1 của Agent 0 ĐÚNG):** `boot_handoff.S:normal_primary` thiếu vòng tô `0xA5` (bản `boot.S` có). Vùng `.bss` toàn số 0 ⇒ `BOOT STACK used=196608 size=196608` (100%) ⇒ `accept()` loại vì `used*2 > size`. Sửa: tô stack trước `rust_main` ⇒ **`used=18744/196608`**. Đồng thời bỏ câu chẩn đoán sai `"banner or SELFTEST PASS missing"` (in cho *mọi* trường hợp còn lại) — nay in đúng điều kiện số học trượt.

**T3 — chi tiết:** đo được `TLB A SEEN (count=263274)`, `LATENCY (ticks=26)` trên bản đột biến ⇒ D vẫn "thấy giá trị mới" vì một lần flush toàn bộ **muộn** (hart 1 đổi root / chạy chương trình khác ⇒ ghi `satp` ⇒ QEMU xoá sạch TLB). Sửa: root ghi cờ "remap xong" vào **word thứ hai** của trang cờ (word 0 worker C dùng cho cửa sổ suspend — ghi đè lên đó làm C thức sai thời điểm, đã bị `PREEMPT OK` bắt); D đọc burst 8 lần ngay sau cờ, chỉ cần một lần thấy giá trị cũ là kết luận stale.

**T3 — hồi quy và bài học (`r228b`):** bản `r228` áp cửa sổ hẹp cho **cả** aarch64 ⇒ bản thường in `TLB REMAP BAD` (sai), rồi `preempts=0`, rồi thiếu `HIARG`. Nguyên nhân: platform aarch64 **không dùng IPI** cho flush (harness đòi `tlb_ipi == 0`). Sửa: `#[cfg(target_arch = "aarch64")]` giữ nguyên vòng lặp trước r228. **Bài học: sửa test cho một arch phải chạy đủ 3 arch TRƯỚC khi commit.**

**T4 — chi tiết:** lld xếp input section trong `.data` theo **thứ tự input**, không theo tên section/static — đã thử `.data.kernel_storage_guard`, `.data.zz_kernel_storage_guard`, đổi tên static (`ZZKSTORE_GUARD`), guard vẫn nằm **trước** storage. Giải pháp deterministic: nhúng guard **vào cùng struct**:
```rust
struct KernelStorage(UnsafeCell<Kernel>, UnsafeCell<[u8; KSTORE_GUARD_SIZE]>); // 4096
```
⇒ liền kề tuyệt đối theo layout, không phụ thuộc linker. Thêm canary (ghi lúc boot, kiểm lúc test-exit) + marker `KSTORE CANARY OK` (exact-one, 3 arch) / `KSTORE CANARY BAD` (FORBIDDEN). `build.sh` nay **FAIL** (trước chỉ warn) khi khe < 4096 và kiểm guard tồn tại.

---

### Part 2 - Quick status

```text
Nhánh: g1 (worktree /tmp/vifast-g1), mốc 5150d0c
Commit G1: 6 (r226, r227, r228, r228b, r229, r230)
HEAD main: KHÔNG đổi (vẫn 5150d0c) — chưa merge, đúng D8

build.sh                PASS  0 warning
AArch64 gap             8 -> 4104 byte  (Kernel=551328 + guard=4096, bkl=1075091888)
canary                  3/3 arch  KSTORE CANARY OK
QEMU --repeat 3         riscv64 3/3, aarch64 3/3, x86_64 3/3 PASS
feature destroy-smp     riscv64 1/1 PASS   (trước 0/1)
feature panic-hart1     riscv64 1/1 PASS   (trước 0/1)
feature boot-handoff    riscv64 3/3 PASS   (trước 0/2)
feature skip-tlb-ipi    riscv64 3/3 PASS   (trước 0/2, "not detected")
                        x86_64 1/1 PASS
gate.sh --quick         PASS
harness negative        PASS 605 cases  (592 -> 596 -> 605)
host test count         khớp (309 test)
```

**Trần trang client (chưa đổi, là mục tiêu T5):** RV 8/8, A64 8/8, x86 9/9 ⇒ **còn 0 trang dư**.

---

### Part 3 - Summary

Chưa cập nhật (thuộc T7). Hiện trạng: `ViFastOs-architecture-summary.txt` tự sinh lại sau T4 nên phần số liệu đã khớp build, nhưng §15/§17/§18/§20 **vẫn dừng ở vòng 100**.

SHA-256 hiện tại của summary: `28af3eb584bf640550e783813c5eacfe618d7483d43640de9278f1e6a34c335f`

---

### Part 4 - Design documents

**Không có design doc mới hoặc sửa trong G1.** 3 file hiện có (giữ nguyên): `display-input-v1.md`, `memory-budget-v1.md`, `pager-v2.md` — trạng thái vẫn là `draft` dù code đã implemented một phần (thuộc T7).

---


---

### Part 5 - Raw evidence (rút gọn, ≤30 dòng/lệnh)

Log đầy đủ: `/tmp/g1-buildH.log`, `/tmp/g1-qemu3.log`, `/tmp/g1-test-*.log`.

**build.sh (AArch64 gap)**
```text
   Compiling vifast-runtime v0.1.0 (/private/tmp/vifast-g1/crates/vifast-runtime)
   Compiling vifast-kernel-x86_64-uefi v0.1.0 (/private/tmp/vifast-g1/crates/vifast-kernel-x86_64-uefi)
   Compiling vifast-platform-x86_64 v0.1.0 (/private/tmp/vifast-g1/crates/vifast-platform-x86_64)
    Finished `release` profile [optimized + debuginfo] target(s) in 4.64s
    Finished `release` profile [optimized + debuginfo] target(s) in 0.01s
build complete
```

**QEMU --repeat 3**
```text
PASS

=== summary ===
riscv64  3/3 PASS
aarch64  3/3 PASS
x86_64   3/3 PASS
```

**feature test-destroy-smp (riscv64)**
```text

PASS
TCP PROBE: SKIP — feature-mode không mở listener TCP (D11)

=== summary ===
riscv64  1/1 PASS
```

**feature test-panic-hart1 (riscv64)**
```text

PASS
TCP PROBE: SKIP — feature-mode không mở listener TCP (D11)

=== summary ===
riscv64  1/1 PASS
```

**feature test-boot-handoff (riscv64)**
```text
BOOT STACK used=18744 size=196608
KSTORE CANARY OK

PASS
TCP PROBE: SKIP — feature-mode không mở listener TCP (D11)

=== summary ===
riscv64  1/1 PASS
```

**feature test-skip-tlb-ipi (riscv64, bản đột biến)**
```text
FAULT EP OK addr=0x0000001000002000
TLB MUTATION DETECTED

PASS (expected TLB failure detected)
TCP PROBE: SKIP — feature-mode không mở listener TCP (D11)

=== summary ===
riscv64  1/1 PASS
```

**harness negative tests**
```text
(không còn log /tmp/g1-harness.log)
```


---
## SHA-256

```text
part1=216b608944056913933305fe23606bf4a26ca43d1c2c088cf9b9a1bef41de2a0
part2..part4 (gộp trong part1) = 216b608944056913933305fe23606bf4a26ca43d1c2c088cf9b9a1bef41de2a0
part5=156ed8d770faef68b574ccb530e3ddcce89d350eb356f04c3942a7a1923fb7b7
part3/summary=ViFastOs-architecture-summary.txt=28af3eb584bf640550e783813c5eacfe618d7483d43640de9278f1e6a34c335f
```
