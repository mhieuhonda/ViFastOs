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

## T12 — D15 (ABI argv) + D14 (multicall `/bin/vf`) — DONE

### Vì sao phải làm trước mọi thứ khác

Exec cũ không có argv. Nó "truyền tham số" bằng cách **vá byte**: root ghi cặp
`(ptr,len)` vào `entry+0x20` và chuỗi vào `entry+0x40` của trang code, còn ảnh
demo đọc đúng offset cứng đó. Hệ quả: (a) chương trình Rust thật không dùng
được ⇒ phải build **3 ảnh riêng** cho `uname`/`uptime`/`free`; (b) không thể
multicall theo `argv[0]`; (c) ghi vào vùng EXEC trước khi map là ràng buộc ngầm.

### Hợp đồng mới (D15) — chi tiết ở `docs/design/exec-argv-abi.md`

- `a0 = argc` (≤ 8), `a1 = argv`, `sp = EXEC_ARGS_BASE` (= `EXEC_STACK_TOP − 256`).
- Khối 256 B trên **trang stack**: 8 con trỏ (64 B) + 192 B chuỗi NUL-terminated.
- Syscall mới `TcbSetArgs = 56`; kernel kiểm `argc ≤ 8`, `argc≠0 ⇒ argv≠0`,
  `argv` canh 8 B, TCB phải `Inactive`/`Exited`.
- `TcbRestart` mang theo `argc/argv` (restart giữ tham số); `TcbConfigure` xoá.

### Kết quả (gate full PASS, QEMU 3 arch × 3 lần)

| Kiểm chứng | Trước r247 | Sau r247 |
|---|---|---|
| Ảnh chương trình ngoài | 3 ảnh × 3 arch (8.5–12.6 KiB mỗi ảnh) | **1 ảnh** `/bin/vf` (8.6–8.8 KiB) + 4 hard link |
| Tham số exec | vá byte trang code | khối argv trên trang stack + `a0`/`a1` |
| Marker argv | `HIARG` (từ vá byte) | `HIARG` (từ argv), `BIN OK args`, `BIN OK uname`, `BIN OK uptime` (EXACT_ONE) |
| Shell fallback | `/<tên>` | `/bin/<tên>` (hard link ⇒ multicall) |

### Hai lỗi thật phát hiện trong vòng này

1. **`sp` ở đỉnh trang làm khối argv bị xoá.** Bản đầu đặt `sp = EXEC_STACK_TOP`;
   x86_64 in `BIN OK args` nhưng `argv:` **rỗng** (thiếu `HIARG`), riscv64/aarch64
   lại đúng. Nguyên nhân: prologue x86 ăn ~256 B ⇒ `rsp` đi xuống dưới
   `EXEC_ARGS_BASE` và ghi đè khối trước khi chương trình đọc. Sửa: `sp =
   EXEC_ARGS_BASE` (khối nằm NGOÀI vùng stack tăng trưởng).
2. **Rò rỉ ở self-test exec của boot.** Ba lần exec mới ở boot giữ TCB/VSpace/
   frame của lần cuối. Đo: `DATA UNTYPED objects=74/86` trước khi sửa,
   `68/86` sau khi sửa; và x86_64 có **1/3 lần đỏ** vì thiếu `MEM OOM OK`
   trước khi sửa. Sửa: `boot_exec_once` gọi `tcb_wait` rồi `destroy_exec` ngay.

### Nợ kỹ thuật ghi nhận (đưa sang T15)

- `--interactive` trước r248 **không hề có tác dụng chặn cổng**: kết quả
  `interactive_results` được thu nhưng không dùng ⇒ input hỏng vẫn xanh. Nay đã
  chặn (mọi lần interactive thất bại đều đỏ) — và ngay lập tức lộ ra 2 lỗi:
  - interactive/tcp/input/gpu boot với **đĩa rỗng** vì `make_disk` truyền
    `"riscv64-ial"` cho `populate_disk` (khóa `TARGETS` không có) ⇒ không có
    chương trình ngoài nào. Sửa: `name.split("-")[0]`.
  - interactive boot `-m 128M` trong khi test chuẩn `-m 256M` ⇒ `MEM OOM BAD`
    hàng loạt. Sửa: dùng 256M.
- Gõ `uname` qua shell ở chế độ interactive vẫn **NOT SEEN**: cửa sổ shell hẹp,
  lệnh gửi lúc prompt tới root khi Untyped đã cạn giữa boot nên root bỏ exec.
  Giữ làm **chẩn đoán** (in `SAW`/`NOT SEEN`), không phải điều kiện đỏ; bằng
  chứng argv cứng nằm ở EXACT_ONE của boot.

## T13 — cwd + chuyển lệnh không trạng thái ra applet + ngân sách trang — DONE

### Mục tiêu và cách đo

Mục tiêu của D13/T13 là **dư trang**: mỗi trang client bị cấp một Frame staging
lúc boot nên nó ăn trực tiếp pool Untyped. Đo bằng `client(...) B = N trang
(ngân sách M)` in ra ở `scripts/build.sh`, kèm SHA-256 file đã strip.

| Thời điểm | riscv64 | aarch64 | x86_64 |
|---|---|---|---|
| Đầu T13 (r248) | 25 088 = 7/8 | 29 024 = 8/8 | 33 192 = 9/9 |
| Sau `cd`/`pwd` + `ps`/`jobs` (r249) | 29 256 = **8/8** | 29 096 = 8/8 | 29 096 = 8/9 |
| Sau `linkers/module.ld` (r249) | **21 008 = 6/8** | **24 944 = 7/8** | **24 968 = 7/9** |
| Dư trang | **2** | **1** | **2** |

### Kết quả lớn nhất: script linker, không phải mã nguồn

`riscv64.ld` (dùng chung cho kernel/root/server/client) `ALIGN(4096)` **từng**
section. Với chương trình người dùng, mỗi khoảng đệm 4 KiB ấy là **một trang
thật trong file ELF**: client có 4 segment căn trang ⇒ 4 trang chỉ để đệm.

`linkers/module.ld` (mới, chỉ dùng cho client + chương trình ngoài) gộp
`.rodata` vào **chính** output section `.text`, còn `.data`/`.bss` chung một
section RW. Kết quả: 2 segment, client **29 256 → 21 008 B** (8 → 6 trang) mà
không xoá một dòng mã nào. Chương trình ngoài: 8 9xx → 6 6xx B.

### Ba cách đã thử và bị dữ liệu bác bỏ

1. **Bỏ `-z separate-code`** cho `build_modules`: **0 byte** thay đổi — script
   linker tự đặt section nên cờ đó không có tác dụng.
2. **Bỏ hẳn `ALIGN(4096)` (căn 16)**: client 21 176 B (6 trang) nhưng boot in
   `MODULE PARSE BAD` + `ROOT PANIC` trên **cả 3 arch**. Nguyên nhân đo được:
   `vifast_elf::parse` làm tròn `memory_end` của mỗi segment lên trang rồi đòi
   các khoảng **không chồng nhau**, vì loader map theo trang — hai segment nằm
   chung một trang bị trả `Overlap`. ⇒ căn trang là ràng buộc của loader, không
   phải thẩm mỹ của linker script.
3. **Chỉ gộp `.rodata` (giữ `.data` riêng)**: đây là phương án chọn. `.text`
   0x4026 → segment R-X kết thúc ở 0x5026; `.data` ở 0x6000 ⇒ file 25 104 B
   (7 trang). Gọt thêm 38 B chú thích trong `help` thì `.text` còn 0x3fxx và
   **toàn bộ ảnh tụt một trang** — đúng bài học "byte ≠ trang", lần này ở chiều
   ngược lại.

### Lệnh nào rời shell, lệnh nào phải ở lại

- `ps` và `jobs` → **applet** `/bin/ps`, `/bin/jobs`: chúng chỉ dùng `TcbInfo`
  (syscall toàn cục, không cần cap) nên chạy được ngoài shell. Đúng D10 (builtin
  chỉ dành cho lệnh có trạng thái).
- `cd`/`pwd` → **builtin**, có `CWD` tĩnh 48 B: cwd là trạng thái **của tiến
  trình shell**, chương trình ngoài không chia sẻ. `cd` chỉ nhận đường dẫn
  tuyệt đối và xác minh bằng `vfs_open` trước khi đổi.
- `df` → **giữ builtin**, ghi vào open-items: `df` cần `STATFS` trên endpoint
  VFS, mà exec hiện **không chuyển cap** cho chương trình ngoài. Muốn đưa `df`
  ra applet phải làm uỷ quyền cap trong ABI exec trước — việc của G2, không
  phải T13.

### Ghi chú vận hành

Đổi nội dung file `.ld` **không** làm cargo relink (linker script không nằm
trong fingerprint của nó). Lần đầu tôi đo nhầm vì artifact cũ: số đo không đổi
dù đã sửa script. Cách đúng: `touch` một file `.rs` của crate đích (hoặc xoá
artifact) rồi build lại.

## T14 — scheduler P1 (K37–K40, K50) — DONE, nhưng khác giả định ban đầu

### Cách làm

Đỏ-trước: viết test host tái hiện đúng trạng thái mà audit mô tả, chạy cho đỏ,
rồi mới sửa. Kết quả đo lại chia các mục thành **hai nhóm**, và nhóm lớn hơn
hoá ra **không phải lỗi**.

### Nhóm 1 — sửa được, có test đỏ-trước: `pick_on` không lọc trạng thái

`best_in_queue` và đường "steal" chọn TCB theo `priority` + `queue_seq` mà
**không xét `state`**. Một entry rác mang trạng thái `Suspended`/`Blocked*`
trong runqueue vẫn được cấp CPU. Nay cả hai đường chỉ chọn `ThreadState::Ready`.

- Test: `r251_pick_on_skips_non_ready_entries`.
- Đã kiểm **mutation**: bỏ lại bộ lọc ⇒ test đỏ (`FAILED`), khôi phục ⇒ xanh.
- Đây là phần K40 thật sự sửa được.

### Nhóm 2 — chứng minh KHÔNG phải lỗi (K37, K38, K39, K50)

Audit giả định "park xong mà `current` chưa được dọn" là lỗi. Đọc mã + đo cho
thấy ngược lại: `switch_blocked` **đọc `current[hart]` để lưu context user**
trước khi `pick_on` thay nó. Dọn `current` ngay lúc park sẽ **mất thanh ghi
người dùng** của task vừa bị chặn.

Bất biến đúng đã tồn tại sẵn trong `check_invariants`: `current[hart]` chỉ được
là `Running` hoặc `Suspended`, và không `queued`. Cửa sổ park→switch nằm trọn
trong một syscall nên không quan sát được từ ngoài.

- Test: `r251_parked_task_leaves_current_after_switch` (sau cú switch: task đã
  park không còn là `current`, không `queued`, giữ `wake_at`).

### Phần K40 "ghi đè `current`" — bị dữ liệu bác bỏ

Đã thử thêm bất biến mạnh: *TCB `Running` phải là `current` của một hart*. Kết
quả: **3 test sẵn có đỏ ngay** — `selftest_passes`,
`per_hart_runqueue_serves_hart2_and_hart3`,
`round69_steal_respects_affinity_and_counts_stats`. Tức kernel hợp lệ **có**
TCB `Running` mà không hart nào giữ `current`.

Đã thử cả cách "sửa" bằng tự chữa: `pick_on` hạ TCB `Running` mồ côi xuống
`Ready` rồi `enqueue`. Kết quả: **treo vô hạn** — nhiều test duyệt runqueue
bằng vòng `while pick_on() …`; tự enqueue lại làm hàng đợi không bao giờ cạn.
Đã bỏ hướng này.

⇒ Ghi nhận bằng test `r251_running_without_current_is_a_valid_state` để lần sau
không ai thêm lại bất biến đó, và để lại đúng lý do trong bảng audit.

### Ảnh hưởng

- `docs/audits/open-items.md`: **39 CLOSED · 3 UNREACHABLE · 4 OPEN**
  (còn K42, K45, K46, K60) — trước vòng này là 34/3/9.
- `check_invariants` **không** thêm ràng buộc mới (đã chứng minh ràng buộc
  tưởng đúng là sai). Giá trị của vòng này nằm ở chỗ **đóng được 5 mục audit
  bằng bằng chứng**, không phải bằng cách nới lỏng.

## T16 — soak 100 × 3 arch tại HEAD cuối — DONE (96,0%)

`python3 scripts/soak.py --iterations 100` tại `a0b6acc`, máy rảnh, không build
song song (đúng yêu cầu "no parallel work" của T16).

```
iterations: 100, elapsed: 2037.4s
riscv64   97/100
aarch64  100/100
x86_64    99/100
```

4 lần hỏng: vòng 25/56/79 (boot chậm làm hết cửa sổ `PREEMPT OK`) và vòng 55
(chỉ probe TCP phía host thua race, marker boot vẫn đủ). Artifact ở
`artifacts/soak/iter-*`.

Đây là **flake đã ghi nhận từ r224**, không phải hồi quy: cùng HEAD đó gate
3×3 PASS 9/9 ngay trước soak. Đề xuất G2: nới cửa sổ `PREEMPT OK` theo thời
gian thực, hoặc cho demo tự chạy lại khi bị bỏ lỡ.

## T17 — xác nhận shell và trang client (r254)

### Builtin của client (đọc từ mã)

`history`, `!!`, `pwd`, `cd <path>`, `help`, `kill`, `time`, `echo <text>`,
`ls`, `cat`, `mkdir`, `rm`, `mv`, `ln`, `stat`, `sync`, `truncate`, `loop`,
`runfs`, `run`, `df`, `fsck`. Mọi tên khác rơi về exec `/bin/<tên>` (multicall
`/bin/vf`): `uname`, `uptime`, `free`, `ps`, `jobs`, `args`.

### Cổng tương tác phải làm lại mới đo được gì

Ba lỗi của chính harness, phát hiện lần lượt bằng số đo:

1. **Đọc hai luồng serial cùng lúc** (socket + logfile của chardev) ⇒ buffer bị
   nhân đôi ký tự (`hheellpp`), marker không bao giờ khớp. Sửa: chỉ đọc logfile,
   socket chỉ để gửi.
2. **Gửi dồn cả kịch bản** ⇒ tràn FIFO UART 16 B của guest, mất byte. Sửa: mỗi
   lần một lệnh, gửi lệnh kế khi đã thấy marker của lệnh trước (D20).
3. **Cửa sổ shell quá hẹp**: guest thoát khi hết việc, `TIMER_TICKS = 20`
   (≈0,2 s). Một lệnh lọt, lệnh sau mất. Sửa: thêm biến thể kernel
   `test-shell-linger` (`TIMER_TICKS = 2000`) dùng **riêng** cho `--interactive`;
   gate và soak vẫn dùng bản chuẩn `TIMER_TICKS = 20`.

### Kịch bản và kết quả (r254)

| Lệnh gửi | Marker | riscv64 | aarch64 |
|---|---|---|---|
| `help` | `help\nhelp time echo ls` | OK | OK |
| `cd /bin` + `pwd` | `pwd\n/bin` | OK | OK |
| `uname` | `BIN OK uname` (lần thứ 2) | OK | OK |
| `ps` | `BIN OK ps` | OK | OK |
| `history` + `!!` | `!!\n!!: ` | OK | OK |

- `uname`/`ps` là **hard link** tới `/bin/vf` nên bảng trên cũng chứng minh
  argv[0] chọn đúng mode khi exec từ shell.
- Marker của `uname` đếm **lần xuất hiện thứ hai** vì `BIN OK uname` đã in một
  lần ở self-test boot; nếu chỉ tìm chuỗi thì test vacuous (đã đo, đã sửa).

### Trang client (D17, đo lại sau thay đổi)

| Arch | ELF | Trang / ngân sách | Dư | SHA-256 (16 ký tự đầu) |
|---|---|---|---|---|
| riscv64 | 21 008 B | 6/8 | **2** | `8974691598237990` |
| aarch64 | 24 944 B | 7/8 | **1** | `53bb172243edb546` |
| x86_64 | 24 968 B | 7/9 | **2** | `e26b29d13aff8f27` |

### Lỗi thật phát hiện thêm (ghi lại, chưa sửa)

`!!` chạy lại lệnh trước: sau khi `!!: <lệnh>` in ra, serial có
`USER FAULT tcb=Some(ObjRef{index:57}) cause=0xf addr=0x1000007ec8` — địa chỉ
nằm trong vùng `.text/.rodata` của **client**, không phải stack. Nghi vấn: đệ quy
`shell_command(&owned[..len])` trong `!!` làm tràn stack client (store page
fault). Cần dựng lại bằng test riêng trước khi sửa (D18).

## T18 — aarch64 lên dư 2 trang (r255) — DONE

### Chẩn đoán bằng số đo (D17)

`nm --size-sort -S` trên client aarch64 so với riscv64:

| Symbol | aarch64 | riscv64 |
|---|---|---|
| `memmove` (compiler_builtins) | 0x240 = **576 B** | 0x38e (khác nguồn) |
| `memset` | 0x11c = **284 B** | 0x72 |
| `memcpy` | 0x110 = 272 B | 0x1b0 |
| `client_main` | 0x17cc | 0x160e |

`.text` aarch64 = 0x4383 (17 283 B), riscv64 = 0x4026 (16 422 B) — chênh
**861 B**, và `memmove + memset` = 860 B. Tức toàn bộ phần chênh đúng bằng hai
hàm builtin mà aarch64 kéo vào.

Vì sao 861 B lại thành 1 trang: file aarch64 = header + segment R-X (0x4383,
kết thúc 0x5383) + segment RW đặt ở 0x6000 ⇒ 24 944 B = 7 trang. Muốn còn 6
trang thì R-X phải kết thúc ≤ 0x5000, tức cắt ≥ 900 B.

### Cách làm

Cung cấp `memset`/`memcpy`/`memmove` bản byte-loop ngay trong client
(`#[no_mangle] extern "C"` với đúng chữ ký `*mut c_void`) để linker không kéo
bản tổng quát của compiler_builtins. Shell chỉ dùng các phép copy nhỏ trong
buffer 32–256 B nên bản byte-loop là đủ và nhỏ hơn nhiều.

Đã thử trước đó và **không** có tác dụng: `--icf=all` (0 byte).

### Kết quả (D17, tái lập được)

| Arch | Trước T18 | Sau T18 | Dư trang |
|---|---|---|---|
| riscv64 | 21 008 = 6/8 | 21 008 = 6/8 | **2** |
| aarch64 | 24 944 = **7/8** | **20 848 = 6/8** | **2** |
| x86_64 | 24 968 = 7/9 | 24 968 = 7/9 | **2** |

⇒ Tiêu chí G1 "client dư ≥ 2 / ≥ 1 / ≥ 2" nay đạt **2 / 2 / 2**.
Gate full PASS 9/9 (log `/tmp/gate-r255.log`).

### Đính chính r259 (D17) — số của r255 không tái lập được

Đo lại tại HEAD `0e60423` bằng đúng lệnh build của `build.sh`:

| Nguồn | aarch64 client (stripped) |
|---|---|
| `d3f2472` (trước r255) | **24 944 B = 7 trang** |
| `0e60423` (sau r255, HEAD) | **24 944 B = 7 trang** |

⇒ Bảng "24 944 → 20 848" ở trên **sai**. Hàm `memset`/`memcpy`/`memmove`
tại chỗ vẫn có trong ảnh (đo `llvm-nm`: `memcpy`=0x20, `memmove`=0x50,
`memset`=0x1c, đều là symbol `T` của client), nhưng chênh lệch aarch64–riscv64
(24 944 − 21 008 = 3 936 B) **không** đến từ compiler_builtins: nó vẫn còn sau
khi thay. Ngân sách thực tế tại HEAD là **6 / 7 / 7 trang**, dư **2 / 1 / 2** —
vẫn đạt sàn G1 (≥2 / ≥1 / ≥2) nhưng aarch64 **không** đạt mục tiêu dư 2.
Bài học: con số trong commit message phải kèm lệnh đo tái lập được (D17) —
r255 đã ghi số của một lần build khác, không phải của cây tại HEAD.

## T21 — bất biến sched ở ranh giới user (r259) — DONE

### Vì sao chỉ kiểm ở "ranh giới"

r251 đã thử đưa "Running ⇔ là `current`" thành bất biến TOÀN CỤC và bị dữ liệu
bác bỏ: ba test sẵn có đỏ (`selftest_passes`,
`per_hart_runqueue_serves_hart2_and_hart3`,
`round69_steal_respects_affinity_and_counts_stats`) vì bên trong một syscall
TCB hiện tại vẫn `Running` trong khi đã nằm chờ đối tác. D19 chốt: chỉ kiểm ở
ranh giới trả về user — nơi trạng thái vừa được `pick_on` thiết lập xong.

### Hợp đồng được mã hoá

`Kernel::check_boundary(hart) -> Result<(), BoundaryError>`:

| Lỗi | Nghĩa |
|---|---|
| `NoCurrent` | hart ở ranh giới mà không có `current` |
| `CurrentNotRunning` | `current` không ở trạng thái Running |
| `RunningAndQueued` | TCB Running còn nằm trong ready queue (lớp K40) |

### Đỏ trước (D18)

6 host test mới, mỗi test là một mutation; đã chạy đỏ-trước rồi xanh:

| Test | Mutation được tiêm |
|---|---|
| `boundary_check_accepts_consistent_current` | không (đường xanh) |
| `boundary_check_rejects_missing_current` | hart không có `current` |
| `boundary_check_rejects_current_not_running` | `current` bị Suspend |
| `boundary_check_rejects_running_but_queued` | `poison_running_queued` |
| `boundary_check_rejects_running_tcb_inside_queue` | TCB khác Running mà vẫn queued |
| `boundary_summary_detects_running_tcb_without_a_hart` | TCB Running không hart nào sở hữu |

### Nối vào đường trả về user

`switch_blocked` và `resume_blocked` gọi `boundary::check::<A>()` ngay sau khi
cài TCB kế tiếp (feature `check-boundary`, chỉ bật ở kernel biến thể). Bản
`check-boundary-mutation` tiêm lỗi **đúng một lần** rồi dọn ngay, nên serial
chỉ có một `BOUNDARY BAD` — đếm được, không nhiễu.

### Kết quả đo (QEMU thật, r259)

| Bản | Marker đo được |
|---|---|
| riscv64 `check-boundary` | `BOUNDARY OK (checks=167884, running=1, harts=2)` |
| x86_64 `check-boundary` | `BOUNDARY OK (checks=95930, running=1, harts=2)` |
| riscv64 `check-boundary-mutation` | `BOUNDARY BAD (hart=0, kind=2)` + `BOUNDARY VIOLATIONS (checks=156788, violations=1, ...)` |
| x86_64 `check-boundary-mutation` | `BOUNDARY BAD` + `violations=1` |

Gate có thêm bước `boundary` chạy cả hai bản (riscv64 + x86_64, cùng phạm vi
với `test-skip-tlb-ipi`; phần logic dùng chung đã có host test).

### Giả định "số TCB Running == số hart" bị dữ liệu bác bỏ

Ở test-exit đo được `running=1` nhưng `harts=2`: hart1 đã idle vẫn giữ
`current` (TCB đang park, không Running). Vậy đẳng thức nguyên văn trong T21
là **sai ở thời điểm exit**. Bất biến đúng — và nay được kiểm — là:

- mỗi TCB `Running` thuộc đúng một hart đang chạy user: `running == running_harts`;
- `running <= harts` (hart idle được phép giữ `current`);
- và `violations == 0` trên toàn bộ phép kiểm ở ranh giới.

Đã đổi phép kiểm theo số đo thay vì giữ giả định.

## T22 — bốn mục audit còn mở (K42, K45, K46, K60) — DONE (r260)

Làm đúng D18: mỗi mục phải có test **đỏ trước**, và đã chứng minh đỏ bằng cách
tạm bỏ bản vá rồi chạy lại (ghi lại kết quả bên dưới).

### K60 — `harts_loaded` "ma"

- **Lỗi**: `check_invariants` chỉ bác bit ≥ `MAX_HARTS`, không bác bit cho hart
  **chưa từng chạy** — tức sổ sách flush có thể ghi một ASID cho hart không tồn
  tại mà không ai biết.
- **Vá**: kernel ghi `harts_seen` (ở `tick`, `pick_on`, `note_vspace_hart_loaded`);
  `check_invariants` đòi `harts_loaded & !harts_seen == 0`.
- **Test**: `check_invariants_detects_phantom_harts_loaded_bit`.
  Bỏ bản vá ⇒ **FAILED** (đã chạy), có bản vá ⇒ ok.

### K46 — `poll` không hạ `pending` của IRQ ⇒ mất ngắt

- **Lỗi**: driver `poll` notification (tiêu thụ word) rồi re-arm thiết bị mà
  không `IrqAck`; `irq_raise` thấy `pending == true` nên **coalesce và không
  signal** ⇒ cạnh kế tiếp mất hẳn.
- **Vá**: `poll` tiêu thụ word thì hạ `pending` của mọi IRQ trỏ tới notification
  đó (quét `MAX_OBJECTS` chỉ khi `word != 0`).
- **Test**: `poll_clears_irq_pending_so_the_next_edge_is_delivered`.
  Tắt nhánh quét ⇒ **FAILED**, bật ⇒ ok.

### K42 — `bound_ntfn` là trường chết

- **Lỗi**: `bind_notification` ghi, `check_invariants` kiểm, nhưng **không chỗ
  nào đọc** để hành động; TCB bind notification xong vẫn `recv` chặn bình thường.
- **Vá**: `recv` poll `bound_ntfn` trước khi chặn — có bit chờ thì trả ngay
  message `LABEL_NOTIFICATION` và tiêu thụ đúng một lần.
- **Test**: `recv_polls_the_bound_notification_before_blocking`.
  Vô hiệu nhánh poll ⇒ **FAILED**, bật ⇒ ok.

### K45 — nghi ngờ "signal xoá word trước vòng lặp làm mất bit" ⇒ **không phải lỗi**

Dựng đúng ca hỗn hợp: hàng đợi `[stale, live]`, `word = 0b0010`, `signal(0b0001)`.
Đo được: waiter sống nhận **0b0011** (đủ bit cũ + mới), waiter treo không bị
đánh thức, `poll` sau đó trả `0` (không sót bit). Test:
`signal_mixed_stale_and_live_waiters_loses_no_bit`.

Kết luận: `value` được tính **một lần** rồi phát cho mọi waiter sống, nên xoá
word trước vòng lặp không làm mất bit nào; nhánh "không ai được phát" còn khôi
phục word. Đóng K45 là **không phải lỗi**, kèm test làm bằng chứng.

### Tổng kết T22

- Audit: **39 CLOSED · 3 UNREACHABLE · 4 OPEN** → **43 CLOSED · 3 UNREACHABLE · 0 OPEN**.
- Kernel host test: **232 → 236** (4 test mới cho K60/K46/K42/K45; trong đó
  2 `#[ignore]` là fuzz nên số chạy được là 234).
- QEMU 3 arch × 3 lần: **9/9 PASS** sau thay đổi IPC (`recv`/`poll`) — quan
  trọng vì K42/K46 đụng đường IPC dùng chung.

## T23 — thiết kế G2: cấp trang & cap cho exec (r260) — DONE (docs)

Tạo `docs/design/g2-memory-and-exec-caps.md` (178 dòng, trần 300):

- **Phần A** — bộ cấp trang tự do: hiện **chưa có** (chỉ Untyped + `retype`);
  so sánh A-kernel vs A-server, đề xuất **đo trước** (3 câu: exec tiêu mấy
  trang, tối đa mấy lần exec, A0 free-list trong root đủ chưa) rồi mới chọn.
- **Phần B** — `df` phải ở lại shell vì exec **không có kênh uỷ quyền cap**;
  đề xuất syscall `TcbGrantCap` với 5 ràng buộc (thu hẹp quyền, không move,
  không ghi đè slot, destroy thu hồi, kiểm liveness) và cấp cho `df` endpoint
  VFS quyền `READ` + badge shell.
- **Phần C** — 5 bước triển khai, 5 rủi ro có dấu hiệu nhận biết, 5 câu hỏi
  mở cần chốt trước khi viết mã.

### Kiểm nhận D21 — soak 20 sau thay đổi boot path (r261)

```
riscv64  20/20 PASS (100.0%)
aarch64  20/20 PASS (100.0%)
x86_64   15/20 PASS (75.0%)
iterations: 20, elapsed: 369.8s
```

5 lần hỏng của x86_64 **đều** là `FS V2 OK` (flake đã bisect về `d3f2472`), không
có loại lỗi mới nào — quan trọng vì T24 đụng đường khởi động AP. Đã merge vào
`main` (ff) sau đó.

## T24 — GDT/TSS theo hart (r261) — DONE (phần cốt lõi)

Trước: `GDT`/`GDT_PTR` cho BSP và `AP_GDT`/`AP_GDT_PTR` cho AP, `ap_entry` dùng
**hằng** `AP_HART` ⇒ trampoline chỉ dựng được cho đúng hart 1 (audit #4 P1-7).

Sau: `GDTS[MAX_HARTS]` + `GDT_PTRS[MAX_HARTS]`; một đường duy nhất
`setup_hart_gdt_tss_idt(hart)` dùng `TSSS/TRAP_STACKS/IST_STACKS` theo hart cho
cả BSP (hart 0) và AP; danh tính hart của AP đi qua **dữ liệu** trong trang
trampoline (ô `0x204`) thay vì hằng số trong mã.

Đóng thêm một mục P2 cùng lúc: `AP_GDT` từng không nằm trong
`kernel_data_ranges`; nay `GDTS` là mảng duy nhất và đã nằm trong danh sách đó.

**Chưa làm (ghi rõ giới hạn):** khởi động hart 2–3. `AP_MAINT_*` hiện giả định
đúng **một** AP (một `AP_APIC_ID`, một ACK counter), nên `-smp 4` cần fan-out
IPI trước. GDT/TSS đã sẵn sàng theo hart nên đây là việc của vòng sau.

## T25 — soak 100 × 3 arch + đóng G1 (r262)

SOAK100_PROGRESS

### Kết quả gate (r262, sau khi hạ MAX_IO)

```
[gate] build: PASS          [gate] persist: PASS
[gate] kernel-test: PASS    [gate] docs: PASS
[gate] tests-present: PASS  [gate] harness: PASS
[gate] fuzz: PASS           [gate] fmt: PASS
[gate] qemu: PASS           [gate] boundary: PASS
all steps passed
```

Lần đầu gate xanh **hoàn toàn** sau nhiều vòng (trước đó x86_64 hỏng ~25%).
`boundary` là bước mới của r259: chạy cả bản `check-boundary` (phải `BOUNDARY
OK`) và bản `check-boundary-mutation` (phải đúng một `BOUNDARY BAD`).

## Việc tiếp theo (bàn giao, r262)

| Vòng | Việc | Vì sao | Điều kiện xong |
|---|---|---|---|
| T26 | Sửa GỐC race FS khi ≥2 hart (hồi quy `FS V2 OK`) | `main` chỉ xanh nhờ hạ `MAX_IO`; race vẫn còn | có test đỏ trước (D18); nâng lại `MAX_IO=256` mà 40/40 lần boot x86_64 xanh |
| T27 | `TcbGrantCap` + đưa `df` ra `/bin/vf` | chứng minh uỷ quyền cap end-to-end; là Phần B của thiết kế G2 | 5 ràng buộc B.2 có test đỏ-trước; `df` chạy từ applet; client không tăng trang |
| T28 | Khởi động hart 2–3 trên x86_64 (`-smp 4`) | `AP_MAINT_*` đang giả định một AP | boot `-smp 4` in `SMP OK (harts=4)`; soak 20 xanh |
| T29 | `!!` làm client fault vào vùng `.text` | lỗi thật đo được ở T17 | có test tái hiện + sửa; `--interactive` vẫn 5/5 |
| T30 | Tìm 3 936 B chênh lệch aarch64 | aarch64 đang dư 1 trang thay vì 2 | aarch64 client ≤ 24 576 B (6 trang) hoặc giải thích được bằng số đo |
| T31 | `PAGER2_LABEL` bị xử lý HAI lần (soak 100, vòng 53, x86_64) | marker `PAGER EVICT OK` in 2 lần ⇒ message giao lặp; nghi cùng gốc IPC với T26 | có test tái hiện; soak 100 không còn vòng nào trùng marker |
| T32 | Không destroy được TCB đã exit khi nó còn là `current` của hart idle (soak 100, vòng 62, riscv64) | `DESTROY DIAG stuck=4` ⇒ `BIN EXEC FAIL`; `object_in_use` coi `current` là đang dùng | chốt ngữ nghĩa (dọn `current` khi idle, hay cho destroy TCB `Exited`) + test |

Công cụ đã có sẵn cho T26: `scripts/x86-smp-probe.py` (tái hiện, `-smp 1|2`,
giữ serial/đĩa), `scripts/dump-root-dir.py` (soi block thư mục gốc), và
`python3 scripts/test-qemu.py` (harness chuẩn, giờ in cả marker bị cấm).

### Kết quả soak 100 × 3 arch (r262, HEAD cuối)

```
riscv64  99/100 PASS (99.0%)
aarch64 100/100 PASS (100.0%)
x86_64   99/100 PASS (99.0%)
iterations: 100, elapsed: 1923.6s
```

**Không còn lần nào hỏng `FS V2 OK`** trong 100 vòng (trước khi hạ `MAX_IO` là
~25%). Hai lỗi còn lại là hai lớp khác, hiếm và đã phân loại:

| Vòng | Arch | Marker | Nguyên nhân |
|---|---|---|---|
| 53 | x86_64 | `PAGER EVICT OK` trùng | root in marker khi nhận `PAGER2_LABEL` lần 2; lần 3 ⇒ in lại ⇒ message bị xử lý hai lần. Nghi cùng gốc IPC với hồi quy FS ⇒ T31 |
| 62 | riscv64 | `DESTROY DIAG stuck=4` + `BIN EXEC FAIL` | 4 TCB đã exit vẫn còn là `current` của hart idle ⇒ `object_in_use` ⇒ `Busy`, không destroy được ⇒ vòng exec sau hết slot ⇒ T32 |

Tiêu chí G1: mỗi arch ≥99% **đạt** (99,0 / 100 / 99,0) nhưng riscv64 và x86_64
sát sàn; không có timeout nào trong 300 lần boot.

## T27 (một phần) — `TcbGrantCap`: uỷ quyền cap cho exec (r264)

Đây là **đường mà exec đang thiếu** (G2 Phần B): `spawn_elf_copy` không cài cap
nào vào cspace của chương trình con, nên mọi lệnh cần VFS (`df`) phải ở lại
trong shell.

Đã thêm `SyscallId::TcbGrantCap = 57` + `Kernel::grant_cap_to_tcb` + wrapper
`vifast_user::tcb_grant_cap`, với 6 host test khoá từng ràng buộc:

| Ràng buộc | Test |
|---|---|
| caller phải có WRITE trên TCB đích | `grant_cap_syscall_requires_write_on_the_target_tcb` |
| slot đích phải trống | `grant_cap_rejects_occupied_destination` |
| quyền bị thu hẹp, không bao giờ GRANT | `grant_cap_strips_grant_right` (đã chứng minh **đỏ** khi bỏ clamp) |
| badge giữ nguyên từ cap nguồn | `grant_cap_keeps_the_source_badge` |
| không chuyển Untyped gốc | `grant_cap_cannot_hand_out_the_root_untyped` |
| derive: caller không mất cap | `grant_cap_installs_a_derived_cap_in_the_target_cspace` |

Kernel host test: 236 → **242** (240 chạy + 2 fuzz `#[ignore]`).

**Chưa làm**: nối vào `exec` rồi chuyển `df` ra applet — cần chốt chính sách
badge (con thừa hưởng badge của shell hay của root) trước, vì badge là `uid`
mà FS server dùng để kiểm quyền file.

## Thí nghiệm T32 (thất bại, đã hoàn nguyên) — ghi để vòng sau khỏi lặp

Định sửa `DESTROY DIAG stuck` (soak 100 vòng 62) bằng cách nhả `current[hart]`
khi hart park (`Kernel::clear_current` + gọi trong `switch_blocked`/
`resume_blocked` khi `pick_on` trả None). **Kết quả: boot chết cả 3 arch**
(timeout, thiếu 37 marker) ⇒ đã `git checkout` hoàn nguyên toàn bộ, build lại,
boot 3/3 PASS.

Suy đoán vì sao sai (để vòng sau đỡ mất thời gian): `current` không chỉ là
"đang chạy" mà còn là **chỗ duy nhất** giữ danh tính task để lưu/khôi phục
context và để `Exit` dọn dẹp; nhả nó ở nhánh park làm mất liên kết đó. Hướng
đúng nhiều khả năng là: cho `destroy_object` chấp nhận TCB `Exited` và tự gỡ
`current` **của chính nó** trong cùng một giao dịch, thay vì dọn `current` ở
đường park.

### Soak 30 sau r264/r265 (kiểm chứng T27 không hồi quy)

```
riscv64  30/30 PASS (100.0%)
aarch64  30/30 PASS (100.0%)
x86_64   30/30 PASS (100.0%)
iterations: 30, elapsed: 657.3s
soak: all iterations passed
```

90/90 lần boot xanh — cả hai lỗi hiếm gặp ở soak 100 (vòng 53, 62) không tái
hiện. Gate full tại r264 cũng PASS 10/10.
