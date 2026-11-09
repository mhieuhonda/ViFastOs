# Tiến độ G1 — "Nền vững" (cập nhật mỗi vòng)

Bắt đầu: 2026-11-09 · Nhánh làm việc: `g1` (worktree riêng `/tmp/vifast-g1`)
Mốc gốc: `5150d0c` (= HEAD `main` lúc mở G1)

| Vòng | Task | Commit | Kết quả | Ghi chú |
|---|---|---|---|---|
| r225 | T0 | — | **BLOCKED** | `main` có tiến trình khác ghi song song: lúc 12:15 `LICENSE` bị tách MIT/APACHE (staged), README/TRADEMARKS/deploy README đang sửa. Không thể "main sạch" theo T0 và không được commit hộ. Theo D8: tách sang worktree riêng, giữ nguyên tree chính. Patch vòng 224 đã lưu: `/tmp/r224-work.patch` (319 dòng, sha256 `9d3472cf…3ace0c`). |
| r226 | T1 | `662ab75` | **DONE** | Check `NET TCP ECHO OK` chỉ áp khi có `TcpProbeListener`; feature-mode in `SKIP` (D11). `harness_spec.tcp_echo_verdict` là nguồn sự thật duy nhất; 4 case âm (592→596). Kết quả: RV `test-destroy-smp` 1/1, `test-panic-hart1` 1/1 PASS (trước 0/1). |
| r227 | T2 | `75f8109` | **DONE** | Giả thuyết #1 của Agent 0 ĐÚNG: `boot_handoff.S` thiếu vòng tô 0xA5 nên `.bss`=0 bị đo thành `used=196608/196608` (100%) → `accept()` loại vì HWM > 50%. Sửa: tô trước `rust_main` ⇒ `used=18744/196608`. Bỏ câu chẩn đoán sai "banner or SELFTEST PASS missing", in đúng điều kiện số học trượt. RV handoff 3/3 PASS. |
| r228 | T3 | `394155c` | **DONE** | Mutation RV phát hiện được. Nguyên nhân: D kết luận OK ngay khi thấy `TLB_B`, nên flush toàn bộ muộn (hart 1 đổi root) che mất đột biến (đo được `A SEEN = 263274`, latency 26 tick). Sửa: handshake qua word 2 của trang cờ + đọc burst 8 lần trong cửa sổ hẹp. RV mutated 3/3, normal 2/2, x86 mutated PASS. |
| r228b | T3 | `77f5123` | **DONE (sửa hồi quy)** | r228 làm **aarch64 đỏ** (bản thường in `TLB REMAP BAD`, rồi `preempts=0`, rồi thiếu `HIARG`) vì áp cửa sổ hẹp của RV lên arch không dùng IPI. Sửa: aarch64 giữ nguyên vòng lặp trước r228. **Bài học:** sửa test cho một arch phải chạy đủ 3 arch TRƯỚC khi commit. |
| r229 | T4 | `0a72abf` | **DONE** | Khe AArch64 **8 → 4104 byte**. lld không cho điều khiển thứ tự `.data` bằng tên section/static (đã thử 3 cách), nên nhúng 4 KiB guard **vào cùng struct** `KernelStorage` ⇒ liền kề theo layout. Canary + marker `KSTORE CANARY OK`/`BAD` (3 arch), `build.sh` FAIL khi khe < 4096. harness 605 cases. |
| r230 | docs | `4066f5e` | DONE | Đồng bộ docs sau T4 (09-testing marker, gen-docs) |
| r231 | docs | `74a8937` | DONE | Báo cáo G1 6 phần (`ViFastOs-report-r230.md`) |
| r233 | T6 | `658d358` | **DONE** | `open-items.md`: 34 CLOSED, 3 UNREACHABLE, 9 OPEN; bỏ 3 assert vacuous `ap_entry` (D11) |
| r234 | T7 | (commit này) | **DONE** | summary §2/§7/§15/§16/§17/§18(D7–D13)/§20; 3 design doc đổi trạng thái + 2 doc mới; check-docs cấm 5 câu drift |
| — | **T5** | — | **CHỜ QUYẾT ĐỊNH** | Cần chọn hướng (a) đa chương trình Rust / (b) blob asm / (c) hoãn |
| r23x | T6 | — | chưa | |
| r24x | T7 | — | chưa | |
| cuối | T8 | — | chưa | soak 100 iter × 3 arch |

## Bài học tích luỹ

1. **Số đo sai còn tệ hơn không đo** (r227): stack chưa tô 0xA5 ⇒ HWM báo 100%, suýt bị đọc thành "tràn stack thật"; câu chẩn đoán "banner missing" còn lái điều tra sai hướng.
2. **Sửa gate phải chạy đủ 3 arch** (r228b): một cửa sổ đo đúng cho RV lại sai cho aarch64 (platform không dùng IPI).
3. **Đừng tin kết luận một lần đo** (r187 → r200): phải mutation hoặc đo lặp.
4. **Linker không nghe lời bằng tên** (r229): kiểm bằng `llvm-nm` rồi mới chọn cách.
5. **Cờ chia sẻ phải chia ô** (r228b): dùng chung word 0 với worker C làm hỏng cửa sổ suspend.

## Trạng thái môi trường

- `main` vẫn bẩn (2 file `crates/` của vòng 224 + file web của tiến trình khác) — **không chạm**.
- Mọi commit G1 nằm trên nhánh `g1`; chưa merge vào `main` (chờ D8 được xử lý).
- Gate nhánh `g1`: `--quick` PASS; QEMU 3/3 arch PASS; harness 605 cases.

## T5 — phân tích trước khi code (đo tại r234, KHÔNG phải ước lượng)

**Hiện trạng ngân sách trang client:**

| Arch | Size (stripped) | Trang | Ngân sách | Dư |
|---|---|---|---|---|
| riscv64 | 29 184 B | 8 | 8 | **0** |
| aarch64 | 29 024 B | 8 | 8 | **0** |
| x86_64 | 33 192 B | 9 | 9 | **0** |

Mục tiêu T5: dư **≥ 2 trang** ⇒ RV/A64 phải xuống ≤ 24 576 B (**giảm ≥ 4 608 B**),
x86 ≤ 28 672 B (**giảm ≥ 4 520 B**).

**Chi phí thật của phần điều khiển lệnh (nm trên client chưa strip, RV):**

| Symbol | Size | Ghi chú |
|---|---|---|
| `shell_command` | **5 192 B** | toàn bộ dispatcher + thân mọi lệnh |
| `shell` | 1 314 B | vòng nhập, lịch sử, ↑/↓ |
| `client_main` | 7 030 B | khởi tạo + IPC + vòng chính |
| `memmove`/`memcpy` | 926 + 432 B | do compiler-builtins |

⇒ Bốn lệnh đọc thông tin (`uname`, `uptime`, `free`, `df`) nằm **bên trong**
`shell_command` 5 192 B. Kể cả bỏ cả 4 thân lệnh, phần tiết kiệm **chưa chắc**
đạt 4,6 KiB — cần đo bằng build loại trừ chứ không được đoán.

**Kết luận (TIMEBOX-NEGATIVE cho vòng này, có lý do):**
1. Chỉ di chuyển 4 lệnh **có thể không đủ** 2 trang; cần thêm: gộp hàm in số
   (`write_u64`, `write_decimal`), bỏ bảng chuỗi trùng, và có thể chuyển cả
   `ps`/`jobs` (nhưng D10 giữ `jobs` vì cần trạng thái shell).
2. Đường ống "chương trình ngoài" **chưa tồn tại**: root chỉ có một mảng byte
   `exec_demo_elf()` 0x180 viết vào `/prog.elf`; **không có** `build.rs`, không
   có `/bin`, shell không có nhánh fallback. Muốn có 1 chương trình ngoài thật
   phải: crate mới → `build.sh` build 3 target + strip → biến môi trường cho
   root `include_bytes!` → root ghi vào FS → shell fallback `runfs /<tên>`.
   Đây là ~4–6 vòng như đã báo trong Part 6 của báo cáo r230.

**Đề xuất làm ngay ở vòng kế (theo thứ tự, mỗi bước có số đo):**
1. Tạo `crates/vifast-bin` (multicall: `uname/uptime/free/df`) — **một** ELF
   duy nhất để giảm chi phí build/embed; `argv[1]` chọn lệnh.
2. Wire `build.sh` + env + `include_bytes!` + root ghi `/bin` (nếu FS chưa có
   `mkdir` thì ghi thẳng `/uname` … và ghi rõ sai lệch D10 trong báo cáo).
3. Client: xóa 4 thân lệnh, thêm fallback `runfs /<line>`; đo lại trang.
4. Nếu chưa đủ 2 trang: tiếp tục gộp hàm in số + chuyển `stat`/`ls` (nếu D10
   cho phép) trước khi nghĩ tới nới ngân sách.

## T5 — phát hiện r238: ảnh root DÙNG CHUNG pool Untyped với hệ

Đã thử hướng "nhúng chương trình ngoài vào ảnh root rồi root ghi ra FS" và
**thất bại có bằng chứng**:

- Thêm 2 ảnh (8 680 + 8 704 B ≈ 17 KiB) vào root ⇒ `DATA UNTYPED` từ **79/86**
  xuống **84/86** page (đúng +5 frame ≈ 20 KiB), tức **mỗi page của ảnh root
  được cấp từ chính pool Untyped của hệ**.
- Hệ quả: 23 marker đỏ (`PAGER RECLAIM OK`, `RUN FS OK`, `HIARG`, `MEM OOM OK`,
  `FS USAGE OK`, …) và `MEM OOM BAD` — vì hết frame cho các demo sau.
- Đã **revert** trong cùng vòng; 3 arch xanh lại (QEMU 3/3 PASS).

⇒ Kết luận: nhúng chương trình vào ảnh root **làm tăng** áp lực bộ nhớ, ngược
mục tiêu T5. Chương trình ngoài phải nằm trên **ảnh đĩa**, không phải ảnh root.

**Cơ chế đúng (đã xác minh là có sẵn):** `vifast-fsutil put-file <disk> <path>
<host-file>` ghi thẳng file vào ảnh FS; `scripts/test-qemu.py::make_disk()` tạo
đĩa 4 MiB mỗi lần chạy. Vậy đường ống T5 là:

1. `build.sh` build 4 ảnh `vifast-bin` (đã có khung ở r237, mới 2 mode).
2. `make_disk()` chạy `fsutil mkfs` rồi `put-file` từng ảnh vào `/uname`,
   `/uptime`, `/free`, `/df` — **không tốn frame Untyped lúc boot**.
3. Client: xoá 4 thân lệnh builtin, thêm fallback `runfs /<lệnh>`.
4. Đo lại trang client; nếu chưa dư 2 trang thì gộp hàm in số + chuyển thêm lệnh.
5. Harness: thêm self-test exec một lệnh ngoài lúc boot (marker `BIN OK <tên>`),
   frame dùng xong phải trả lại (kiểm bằng `EXEC MEM STABLE`).

Việc này còn ~2–3 vòng; khung build đã sẵn nên không phải làm lại từ đầu.

## T10 — đo lại kích thước client sạch (D17), giải thích chuyện "trùng byte"

Đo bằng dựng lại từ `cargo clean` trong worktree riêng `/tmp/vf-before` (mốc `8deeb02`,
trước r241) và trên `main` sau r241. Mỗi số kèm SHA-256 của file đã strip.

| Arch | text trước → sau | rodata trước → sau | ELF trước → sau | SHA trước / sau |
|---|---|---|---|---|
| riscv64 | 0x40ce (16 590) → 0x3fec (16 364) **−226** | 0x6de → 0x6b8 | 29 184 → **25 088** | `e24267b3…` / `070b3051…` |
| aarch64 | 0x4fa4 (20 388) → 0x4e74 (20 084) **−304** | 0x640 → 0x62a | **29 024 → 29 024** | `521ced8d…` / `9db831e4…` |
| x86_64 | 0x4c74 (19 572) → 0x4b84 (19 332) **−240** | 0x656 → 0x641 | **33 192 → 33 192** | `c2038f28…` / `a0e7c6c7…` |

**Trả lời Agent 0 (điểm 1):** không phải đo nhầm artifact, và không phải code không bị loại.
SHA **khác nhau** ở cả 3 arch ⇒ binary thật sự đã đổi. Nhưng phần co lại chỉ **226–304 byte**,
trong khi **một trang là 4 096 byte**:

- riscv64: nhờ 226 byte mà vượt được biên trang ⇒ **−1 trang** (8 → 7).
- aarch64/x86: co 240–304 byte nhưng **không vượt biên trang** ⇒ số trang và kích thước file
  **không đổi một byte**.

⇒ Bài học đo lường (đưa vào D17): **"byte" không phải đại lượng quyết định, "trang" mới là.**
Muốn giảm 1 trang phải cắt ≥ phần dư tới biên trang kế tiếp, hoặc cắt cả trang.
Từ nay mỗi lần đo ghi: `offset trong trang của PT_LOAD cuối` + số trang thật.

## T9 — main về trạng thái có bằng chứng (DONE)

- **Gate full trên `main` PASS** (`/tmp/main-gate-full3.log`): build, kernel-test,
  tests-present, fuzz, qemu (3 arch × 3), persist, docs, harness, fmt — tất cả PASS.
  Trước đó có 1 lần FAIL do flake `input` riscv64 (key chưa tới shell) — đúng loại
  flake đã ghi ở r224, không phải lỗi code.
- **D8:** nhánh `web` giữ `deploy/`; trên `main` dùng `git rm -r --cached deploy/`
  (giữ file trên đĩa để tiến trình web đang chạy không bị xoá đột ngột) + `.gitignore`.
  Bản sửa dở `index.html` đã lưu `/tmp/web-index-dirty.html`
  (sha256 `978e0f31…`).
- **`ipc.rs`:** fix P0 reply-orphan **chưa từng ở main** — chỉ nằm trên `wip/r224`.
  Nay đã đưa vào (`r242`) kèm **host test `reply_with_bad_cap_slot_restores_caller`**,
  đã chứng minh **đỏ trên code cũ** (cargo test FAILED) rồi xanh sau fix.
  Đây là chỗ docs §18 mục 48 mô tả sai — nay đã đúng.

## T10 — đo lại kích thước (DONE): xem bảng ở trên; kết luận "byte ≠ trang".

## T11 — giảm kích thước bằng cấu hình build

| Bước | Thay đổi | RV | A64 | x86 | Kết luận |
|---|---|---|---|---|---|
| 0 | baseline (r241) | 25 088 = **7/8** | 29 024 = **8/8** | 33 192 = **9/9** | |
| 1 | `opt-level = "z"` cho `profile.root-task` | 25 088 = 7/8 | **24 928 = 7/8** | **29 096 = 8/9** | **CÓ LỢI — giữ** (`r244`) |
| 2 | loại `core::fmt` | — | — | — | **KHÔNG ÁP DỤNG**: `nm` cho thấy chỉ còn `panic_fmt` 16 B, không có fmt machinery |
| 3 | panic handler không format | — | — | — | đã ở trạng thái tối thiểu (panic = abort) |
| 4 | linker: gộp rodata | chưa thử | — | — | để vòng sau |

Chuyển `free` thành applet (`r245`): client **không đổi trang** (chỉ cắt ~150 B,
cần ≥512 B mới vượt biên trên riscv64) — **ghi nhận là kết quả âm có số đo**.

**Hiện tại:** RV 7/8, A64 7/8, x86 8/9 ⇒ **dư 1 trang mỗi arch** (mục tiêu 2, sàn 1).
Cần thêm 1 trang: theo kế hoạch là T12 (ABI argv) + T13 (df/ps cần cap) — không thể
đạt bằng cắt chuỗi nhỏ lẻ.
