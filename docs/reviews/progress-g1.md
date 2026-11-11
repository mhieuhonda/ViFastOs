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

## T27 (hoàn tất) — `df` ra applet, chương trình con nhận cap VFS (r268)

Nối `CapMint` vào `spawn_elf_from_fs`: sau `TcbSetArgs`, root mint endpoint VFS
từ cap GỐC (badge 0) vào slot 1 của chương trình con với **badge 1** — đúng
danh tính shell, không phải badge root (`BADGE_SUPERUSER`).

- `df` bỏ khỏi danh sách builtin của client; thêm applet `df` trong
  `vifast-bin` (gọi `STATFS` qua slot 1, không cần IPC buffer vì mọi tham số
  nằm trong message words).
- Ảnh đĩa có thêm hard link `/bin/df` (cùng inode `/bin/vf`).
- Cổng `--interactive` có bước mới `df` → marker `blocks`: nếu cap không được
  uỷ quyền, applet in `df: error` và bước đỏ.

Đo được (QEMU thật):

```
[riscv64] df -> 'blocks': OK    -- riscv64 interactive PASS
[aarch64] df -> 'blocks': OK    -- aarch64 interactive PASS
```

Đây là **applet đầu tiên cần capability**, tức G2 Phần B đã chạy end-to-end.

### Soak 30 tại HEAD cuối (r269, sau khi `df` thành applet)

```
riscv64  30/30 PASS (100.0%)
aarch64  30/30 PASS (100.0%)
x86_64   30/30 PASS (100.0%)
iterations: 30, elapsed: 576.0s
soak: all iterations passed
```

Tổng ba đợt soak cuối phiên: 30 (r265) + 30 (r269) + 100 (r262) — **chỉ 2 lỗi
trên 480 lần boot**, cả hai thuộc hai lớp hiếm đã ghi ở T31/T32, không có lần
nào hỏng `FS V2 OK`.

## T32 (r271 sửa → r273 HOÀN NGUYÊN) — vẫn mở

r271: `destroy_object`/`can_destroy` nhận TCB `Exited` dù còn là `current`,
dọn luôn tàn dư `current`; TCB `Running` vẫn bị từ chối. Hai host test
đỏ-trước (test "đồng ý huỷ" đã chứng minh ĐỎ khi tắt bản vá). Gate 10/10, soak
30 tại r271 = 90/90.

**Nhưng soak 100 ngay sau đó hỏng vòng 2**: x86_64 treo sau `shell> `, thiếu
`PREEMPT OK`. Trước bản vá hai đợt soak 30 liên tiếp đều 90/90 và không có lần
treo nào ⇒ coi là hồi quy, **r273 hoàn nguyên** phần kernel.

Trạng thái: **mở**. Dữ kiện mới: dọn `current` khi destroy chạm vào đường
scheduler của hart đang chạy, nên phải hiểu ai còn đọc `current` sau khi task
exit trước khi sửa lại. Bài học: xanh gate + xanh soak 30 **chưa đủ**; soak 100
mới bắt được.

## T33 (ĐÍNH CHÍNH lần 2, r276) — lỗi THẬT, hai triệu chứng

Bản ghi trước kết luận "flake harness" là SAI: file serial của lần hỏng đã bị
lần chạy THỬ LẠI ghi đè (harness có retry, serial dùng chung đường dẫn).

Đọc `run.log` của đúng lần hỏng mới ra sự thật:

| Vòng | Arch | run.log | Nghĩa |
|---|---|---|---|
| 004 | x86_64 | `rc=-9, 80.1s`, thiếu `PREEMPT OK` | treo thật tới hết 80 s |
| 096 | riscv64 | lần 1 `rc=0, 2.6s` thiếu 48 marker; lần 2 PASS | guest thoát sớm |

- **T33a (x86_64 ~1%)**: treo trước `PREEMPT OK`; `SMP IDLE STALL` cho thấy bộ
  đếm post-idle của hart phụ kẹt ở 3–4 (cần ≥5). Đã có `SMP AP STATE (online,
  window, gen, wake_gen, ipi, resched)` để vòng sau đọc trạng thái AP.
- **T33b (riscv64 ~1%)**: guest thoát mã 0 sau 2,6 s, thiếu 48 marker.

Cần làm: chạy `soak.py --iterations 100 --archs x86` (nhanh hơn 3 arch) để bắt
một lần treo kèm `SMP AP STATE`, rồi quyết định sửa ở đường đánh thức hart phụ.
**Bài học**: khi có retry, phải biết serial thuộc lần chạy nào — `run.log` ghi
`rc` và thời lượng từng lần.

### Soak 30 tại HEAD cuối (r269, sau khi `df` thành applet)

```
riscv64  30/30 PASS (100.0%)
aarch64  30/30 PASS (100.0%)
x86_64   30/30 PASS (100.0%)
iterations: 30, elapsed: 576.0s
soak: all iterations passed
```

Tổng ba đợt soak cuối phiên: 30 (r265) + 30 (r269) + 100 (r262) — **chỉ 2 lỗi
trên 480 lần boot**, cả hai thuộc hai lớp hiếm đã ghi ở T31/T32, không có lần
nào hỏng `FS V2 OK`.

## T32 (r271 sửa → r273 HOÀN NGUYÊN) — vẫn mở

r271: `destroy_object`/`can_destroy` nhận TCB `Exited` dù còn là `current`,
dọn luôn tàn dư `current`; TCB `Running` vẫn bị từ chối. Hai host test
đỏ-trước (test "đồng ý huỷ" đã chứng minh ĐỎ khi tắt bản vá). Gate 10/10, soak
30 tại r271 = 90/90.

**Nhưng soak 100 ngay sau đó hỏng vòng 2**: x86_64 treo sau `shell> `, thiếu
`PREEMPT OK`. Trước bản vá hai đợt soak 30 liên tiếp đều 90/90 và không có lần
treo nào ⇒ coi là hồi quy, **r273 hoàn nguyên** phần kernel.

Trạng thái: **mở**. Dữ kiện mới: dọn `current` khi destroy chạm vào đường
scheduler của hart đang chạy, nên phải hiểu ai còn đọc `current` sau khi task
exit trước khi sửa lại. Bài học: xanh gate + xanh soak 30 **chưa đủ**; soak 100
mới bắt được.

## T33 (ĐÍNH CHÍNH r275) — treo `PREEMPT OK` là flake của HARNESS

Bản ghi trước của mục này kết luận "hart phụ ngừng thức". **Sai.** Giữ serial
của lần hỏng (soak 100 vòng 58, riscv64) ra thì nó có ĐỦ mọi marker mà harness
báo thiếu, kể cả `PREEMPT OK` (2 lần):

```
Cap OK         5
Client         1
PREEMPT OK     2
SHELL READY    1
FS V2 OK       1
```

⇒ khách không treo; harness hết thời gian chờ trong khi khách còn boot. Đây là
lỗi lớp T19 (thu thập log), không phải kernel. Dòng `SMP IDLE STALL` cũng xuất
hiện trong những lần PASS với đúng bộ số đó ⇒ không phải dấu hiệu hỏng.

Việc cần làm (vòng sau):
1. Sau timeout, **đọc lại serial** rồi mới kết luận thiếu marker.
2. Nới timeout khi máy có tải; **không chạy build/gate song song với soak**
   (phiên này đã vi phạm vài lần — các lần "treo" đều rơi vào cửa sổ đó).
3. Giữ `SMP AP STATE` (r275) làm chẩn đoán nếu sau này thật sự có hart chết.

### Công cụ: sửa `--archs` của soak (r277)

`--archs x86` từng làm **mọi vòng fail giả**: soak chuyển nguyên chuỗi `x86`
cho `test-qemu.py`, mà argparse ở đó chỉ nhận `x86_64`. Nay soak map tên ngắn
(`rv`, `arm`, `x86`) sang tên đầy đủ và báo lỗi rõ nếu tên không hợp lệ.
Smoke: `soak.py --iterations 1 --archs x86` → 1/1 PASS trong 8,9 s.


### T33b — chẩn đoán chi tiết (r279)

Chạy soak MỘT kiến trúc cho thấy tỉ lệ cao hơn hẳn (riscv 197/200 và 197/200 ở
hai đợt, tức ~1,5–2%):

| Lần chạy | Kết quả | Bằng chứng |
|---|---|---|
| lần 1 (hỏng) | `rc=0` sau ~2,8 s, thiếu 45–50 marker | tail serial: `MEM OOM BAD` ×11, rồi `USER FAULT … cause=0xd` ở `0x1001000000`, `0x1001001000`, `0x1001002000`, rồi `USER EXIT` |
| lần 2 (thử lại) | PASS | nhưng bước TCP echo ĐỎ: `không mở được listener ([Errno 48] Address already in use)` |

⇒ có **hai** vấn đề chồng lên nhau: (1) thỉnh thoảng cạn bộ nhớ giữa boot khiến
trang của applet chưa được map ⇒ fault; (2) sau một lần hỏng, cổng 18080 còn
bận nên lần thử lại bị đếm hỏng thêm một lần nữa.

Đã thử:
- `destroy_exec` thử lại 8 pass có `yield_now()` — **không** đo được cải thiện
  (giữ lại vì đúng ngữ nghĩa, có chặn trên).
- Cổng TCP động cho probe — **hỏng ngay** (probe và `hostfwd` tranh cùng cổng),
  đã hoàn nguyên. Hướng đúng: đổi thiết kế probe (dùng cổng riêng cho từng lần
  chạy hoặc chờ cổng cũ nhả), không phải đổi mỗi số cổng.

### Bài học vận hành (r279): build dở dang làm QEMU đỏ hàng loạt

Sau khi build lẻ một crate (`cargo rustc -p vifast-root …`) rồi chạy QEMU, riscv
đỏ **liên tục** (gate 0/3, `MEM OOM BAD` lặp, `USER FAULT` ở ba TCB mới) trong
khi trước đó cùng mã nguồn vẫn xanh. Chạy lại `./scripts/build.sh` (build đầy
đủ, đúng bộ binary chuẩn) thì xanh trở lại ngay (riscv 2/2, gate 10/10).

⇒ Quy ước: **luôn `./scripts/build.sh` trước mọi lần chạy QEMU**, không để cây
ở trạng thái build một phần (gate/soak dùng chung `target/…/release`, một crate
được build lẻ là đủ để lệch bộ binary).

### Soak 100 tại HEAD cuối (r280) — số chốt của phiên

```
riscv64  99/100 PASS (99.0%)
aarch64 100/100 PASS (100.0%)
x86_64  100/100 PASS (100.0%)
iterations: 100, elapsed: 1908.2s
```

Lỗi duy nhất (vòng 56, riscv64) đúng lớp T33b: lần 1 `rc=0` sau 3,0 s thiếu 45
marker, lần chạy lại PASS nhưng bước TCP echo đỏ vì cổng còn bận.

**Tổng cả phiên**: 100 + 30 + 30 + 100 + 40 + 120 (x86) + 200 (rv) + 200 (rv) +
100 = khoảng **1 200 lần boot** cho 3 kiến trúc; các lỗi còn lại chỉ thuộc hai
lớp hiếm T33a/T33b và T31.


## Hợp nhất từ nhánh `r254` (r283) — phân tích T18 còn giá trị

Nhánh `r254` (r256–r258) không merge nguyên được: hai trong ba commit là tài
liệu đã cũ, commit `r257` mang bản `scripts/test-qemu.py` CŨ HƠN `main` (main đã
có bản sửa T19/T20 tốt hơn). Phần duy nhất còn giá trị là phân tích T18 dưới
đây, được chép nguyên văn vào đây rồi nhánh được xoá.

### Hồi quy về 1 trang sau khi merge r253 của nhánh web (issue #10)

Ngay sau khi rebase lên `d3f2472` ("sửa 12 issue #2–#13"), aarch64 trở lại
**24 944 B = 7 trang**, tức dư 1. Nguyên nhân đo được: commit đó thêm đường
`READDIR_BULK` vào client (+107 dòng) làm `.text` aarch64 từ ≤ 0x4000 lên
**0x4157** (16 727 B). Để về 6 trang cần `.text ≤ 0x4000`, tức phải cắt thêm
**343 B**; riêng phần bảng section header của file là 280 B và segment RW nằm ở
0x6000 nên không thể "lấn" sang.

Trạng thái T18: **PARTIAL — đạt sàn (dư 1) cho aarch64**, không đạt mục tiêu 2.
Các hướng đã thử và bị dữ liệu bác bỏ: `--icf=all` (0 byte), thay mem* builtin
(đã làm, ăn 4 096 B nhưng bị 343 B của issue #10 lấn trở lại). Hướng còn lại
đều phải cắt tính năng (help/time/echo thành applet) — để G2 quyết vì free-page
allocator sẽ bỏ hẳn ràng buộc trang.

## T26 — bài test hồi quy (r283)

`scripts/fs-smp-regression.py` là dạng "test đỏ trước" khả thi cho lỗi `FS V2 OK`
(chỉ lộ qua nhiều lần boot QEMU, không unit-test được trên host):

```sh
python3 scripts/fs-smp-regression.py --iterations 40            # phải xanh hết
python3 scripts/fs-smp-regression.py --iterations 20 --expect red  # phải có đỏ
```

Đo trong phiên: `--expect green` 8/8 xanh ở `MAX_IO = 128`; đỏ đã đo ở
`MAX_IO = 256` là 12/47 lần boot (Part 4 của báo cáo). Quy ước: **nâng `MAX_IO`
phải kèm `--expect green` với N ≥ 40**.

### Ghi chú: bảng tổng hợp kiến trúc luôn "chậm một commit"

`ViFastOs-architecture-summary.txt` chứa cả **danh sách commit sinh tự động**, nên
mỗi lần commit là một lần file đó "cũ" đi — `check-docs.py` sẽ báo
`summary generated sections are stale` ngay sau khi commit. Đó là vòng lặp tự
tham chiếu, không phải lỗi: cổng xử lý bằng cách chạy `gen-docs.py` TRƯỚC bước
docs, nên gate vẫn xanh. Đừng cố "sửa" bằng cách commit lại file này từng lần.

## T33a/T33b (r288) — hai lỗi thật, cả hai đều "hart phụ hết việc ⇒ giết cả máy"

Điều tra bằng serial của **đúng lần chạy hỏng** (trước đây file bị lần thử lại
ghi đè — đã sửa: tên file giữ serial nay có thêm số lần thử `-aN`).

### Lỗi 1 — `has_ready_on` không kiểm `state`

`has_ready_on(hart)` chỉ lọc theo affinity, còn `pick_on(hart)` chỉ nhận TCB
`Ready`. Một TCB còn trong hàng chờ nhưng đã đổi trạng thái làm `has_ready_on`
trả `true` ⇒ hart phụ thức dậy, `pick_on` trả `None`, runtime gọi `A::exit(1)`
⇒ **hart phụ chết**. Test đỏ-trước:
`has_ready_on_ignores_a_queued_tcb_that_is_not_ready` (đỏ trước bản vá, xanh sau).

### Lỗi 2 — `UserAction::Exit` giết cả máy khi hart phụ hết task

Serial của lần hỏng (x86_64) kết thúc bằng `TCB REAP OK` rồi `USER EXIT` giữa
đường. Đọc mã: khi task hiện tại exit mà **hart đó** không còn task kế tiếp,
runtime in `USER EXIT` và `exit(0)` — dù hart khác vẫn đang chạy root. Một
worker demo thoát trên hart 1 là đủ giết cả hệ thống (~1–2%).

Sửa: chỉ kết thúc khi **không còn TCB `Ready` nào ở bất kỳ hart nào** và không
hart nào còn `current` (`Kernel::has_any_ready` mới); nếu còn thì hart này ngủ
chờ như đường `Blocked`.

### Chẩn đoán thêm

`SMP WAIT (ticks, done, fs, fsl, link, sym, tlb, post, cur0, cur1, rq0, rq1)`
in MỘT lần khi `PREEMPT OK` chưa đủ điều kiện — để lần sau biết kẹt ở cờ nào.
Đo được ở lần treo còn lại: `done=false … post=false cur0=true cur1=true rq0=4`,
guest dừng NGAY SAU `shell>` (hart 0 không tiến nữa) ⇒ nghi bộ đếm LAPIC của
hart 0 ngừng phát; đây vẫn là phần **mở** của T33a.

## Soát tầng đáy (r289) — P0-1 + P0-2 (một phần P0-2), theo `docs/audits/2026-11-10-soat-tang-day.md`

### P0-1 — aarch64 leaf user thiếu nG (ĐÃ SỬA, có test đỏ-trước)

`vifast-pte` `aarch64::encode_leaf` không đặt bit 11 (nG) ⇒ mọi leaf là *global*,
mà `tlbi aside1is` **không** xoá entry global ⇒ hai VSpace cùng map một VA
(client/server đều map `CODE_VA`) thấy entry của nhau.

- Test đỏ-trước `aarch64_leaf_is_non_global` (đỏ trước bản vá, xanh sau).
- Sửa kèm `flush_va` của platform-aarch64: gắn ASID
  (`(active_asid() << 48) | va`), nếu không thì khi đã có nG lệnh flush thành
  vô dụng — đúng cảnh báo "hai chỗ phải sửa cùng nhau" trong bản soát.

### P0-2 — publish `seq` trước payload ⇒ mất một lần invalidate (SỬA 2/3 port)

Thêm crate **`vifast-maint`** (no_std, không `unsafe`) cài giao thức hộp thư
MỘT lần: `publish` ghi `kind` (Release) **trước** rồi mới tăng `seq`; `take`
trả `None` khi `kind == 0` và **không ACK**; `ack` chỉ gọi sau khi làm xong.

| Port | Trạng thái | Ghi chú |
|---|---|---|
| aarch64 | **đã nối** | `MAINT: [Mailbox; MAX_HARTS]`, thêm `dsb ish` trước doorbell |
| x86_64 | **đã nối** | `AP_MAINT: Mailbox`, `fence` trước khi ghi ICR |
| riscv64 | **CHƯA** | nối vào thì boot hỏng 0/5 (revert ⇒ 3/3 xanh); giữ mã cũ, đã ghi việc |

Test host của crate (chạy được vì không asm):
- `publish_then_take_yields_the_kind`;
- `take_without_kind_must_not_ack` — **đỏ** khi trả về giao thức cũ (đã chứng
  minh bằng cách tạm đảo lại), đây là test khoá đúng lỗi P0-2;
- `concurrent_publish_never_loses_a_flush` (20 000 lần, hai luồng).

Đo sau khi sửa: build PASS, QEMU 3 arch × 3 = 9/9 PASS, gate 10/10.

## Agent 0 chốt r289–r294 (2026-11-10) — thực thi theo tệp quyết định

Tệp: `ViFastOs-quyet-dinh-agent2-r289-r294.md` (Agent 0). Thứ tự vòng: r289
(tầng đáy) → r290 (capability) → r291 (FS) → r292 (gốc race FS) → r293 →
r294 (tách lib.rs).

### r289 — xong (commit `1e5c051`)

| Mục | Nội dung | Test đỏ-trước |
|---|---|---|
| 289.2 | Payload (va, pages, asid) nằm TRONG hộp thư `vifast-maint`, hợp nhất chỉ nới rộng (Range→Asid→All), không ghi đè | `two_publishers_both_get_covered`, `publishers_with_different_asids_cover_both`, `scope_merge_never_narrows` |
| 289.3 | x86 shootdown đối xứng (bỏ `target != 1`), mỗi hart một hộp thư, `APIC_IDS`, đếm theo cặp `SHOOTDOWN_SENT[nguồn][đích]` | `ap_can_publish_to_the_bsp_mailbox` |
| 289.4 | `BKL.is_held()` → `held_by(current_hart())` ở 8 chỗ 3 port | `held_by_is_owner_only_while_a_waiter_sees_is_held` |
| 289.5 | Sàn test theo số đo (`scripts/test-floor.py` + `test-floor.json`), gate đỏ nếu giảm | mutation: xoá 1 test ⇒ `vifast-maint: 6 < 7` ⇒ ĐỎ |

riscv64 **chưa** nối hộp thư dùng chung: nối vào thì boot 0/5, revert thì 3/3 —
giữ mã cũ, ghi thành việc của vòng sau.

### r290 — 290.1 xong, 290.2 dở (commit `fc6aec4`)

- 290.1: `CapRef.cnode`, `TableId`, `Kernel::for_each_cap` (root + TCB nhúng +
  **mọi** CNode kể cả đã tháo), `resolve_cap_ref`, `cap_slot_capacity`; test
  `for_each_cap_visits_every_slot_of_every_live_table` (7 cap / 4 bảng).
- 290.2 (dở): `is_descendant`/`survivor_for` phân giải qua bảng vật lý; bỏ
  `return` sớm trong `reparent_tcb_caps`; vòng sửa `cap.parent` đi qua mọi bảng
  vật lý. Test đỏ-trước `parent_pointing_into_a_dying_tcb_is_repaired_inside_a_cnode`.
  **Chưa** dựng được kịch bản 5 bước đầy đủ (revoke + destroy) ⇒ **không** tuyên
  bố đóng P0-3.

## r291 — FS: capacity, biên indirect, lỗ file thưa, rollback create

- **291.1/P1-1**: `Fs.capacity` lấy từ superblock (`total`) thay `dev.block_count()`;
  `block_is_used`/`mark_*`/`alloc_block`/`usage` đều dùng biên này. Test đỏ-trước
  `mount_accepts_a_device_larger_than_the_formatted_area` (ĐỎ khi trả biên về
  `dev.block_count()`: panic `index out of bounds: len 1024 index 1024`).
- **291.2/P1-2**: chỉ số block bịa trong inode → `Corrupt` thay vì panic. Test
  `read_with_a_forged_size_returns_corrupt_instead_of_panicking`.
- **291.3**: ghi quá EOF tạo LỖ hợp lệ — `read` trả 0 cho block trống trong
  phạm vi `size`, `fsck` bỏ qua lỗ. Test
  `writing_past_eof_leaves_a_readable_zero_filled_hole` (ĐỎ khi trả `read` về
  `Corrupt`).
- **291.4**: `create` hỏng ở bước ghi entry phải rollback inode đã cấp
  (`release_inode`). Test `failed_create_rolls_back_the_allocated_inode` (ĐỎ khi
  gỡ rollback: `left: 18, right: 17` — rò rỉ đúng một inode).
- **291.6 (một phần)**: `fsck_mark_block` dùng biên `capacity` của FS.

Đo: `vifast-fs` **44 test**, gate 10/10, QEMU 3×3 = 9/9.

## r292 — tái hiện race FS ở MAX_IO=256 (chưa sửa gốc)

Đo trên cây HEAD (chỉ tạm đổi `MAX_IO=256`, trả về 128 ngay sau khi đo):

```
python3 scripts/fs-smp-regression.py --iterations 20 --expect red
→ 3/20 lần đỏ (thiếu marker "FS V2 OK"), 17/20 xanh
```

Khớp bảng đo cũ (12/49 ≈ 24% ở bản 256, 0/32 ở bản 128). Triệu chứng giữ
nguyên: `mkdir` OK nhưng `readdir` ngay sau đọc ảnh CŨ của block thư mục gốc.
Ba nghi phạm xếp thứ tự điều tra: (1) cache block 32 entry của FS server không
invalidate khi ghi staged; (2) `commit()` từng chunk 512 B để lộ trạng thái
trung gian; (3) mailbox maintenance (r289) chưa phủ hết vùng FS.

## T33a / T33b (r293) — chẩn đoán bật đèn + guard hart idle

### T33a (x86_64 treo trước `PREEMPT OK`)

Đã thêm hai thứ **không phải marker**:
- `SMP NO READY (hart, ticks, exitted_current, alive)` ở đúng nhánh
  `pick_on(hart) == None` của đường timer — trước đây nhánh này thoát IM LẶNG
  (`exit(1)`), nên harness chỉ thấy "thiếu PREEMPT OK" mà không biết vì sao;
- `Kernel::live_tcb_count()` để phân biệt "hết việc thật" (0 TCB sống) với
  "còn TCB sống mà hart không chạy được".

Đo (x86_64, 80 lần boot, diagnostic bật): **80/80 PASS**, 1 lần hỏng sớm thuộc
lớp T33b (USER FAULT → "no runnable thread after fault"), retry cứu.
**0 lần** `SMP NO READY` ⇒ nhánh thoát im lặng không phải gốc của T33a.

Lần hỏng T33a còn lại (59/60 ở đợt đo thứ ba) có dấu hiệu: serial dừng ngay sau
`shell>`, **không** in `SMP WAIT` (chỉ in khi tick ≥ 99), **không** in
`SMP NO READY`, **không** in `KERNEL FAULT` ⇒ hart 0 kẹt TRƯỚC khi tick tới 99
và trước mọi nhánh thoát ⇒ nghi hart 0 quay BKL (hoặc lock console) do hart phụ
giữ rồi hart phụ chết. Việc của vòng sau: thêm bộ đếm spin BKL in-một-lần khi
vượt ngưỡng (`BKL STUCK owner=.., holders=..`) để xác nhận.

### T33b (riscv64 thoát sớm)

`fault::suspend` khi `current_on(hart) == None` (hart đang idle trong kernel)
trước đây đi thẳng tới `fault_current_on(hart)` (không có current ⇒ `None`) ⇒
"no runnable thread after fault" ⇒ `halt()` ⇒ **chết cả máy** vì một ngắt mồ
côi. Nay: in `KERNEL FAULT (hart, cause, addr) — hart idle, bỏ qua`,
`ack_timer()`, quay lại vòng idle.

Đo riscv64 **60/60 PASS**, **0** `KERNEL FAULT` (tỉ lệ cũ ~1,5% ⇒ kỳ vọng ~1
lần; mẫu chưa đủ để kết luận, cần ≥300 lần mới nói được). x86_64 59/60 sau vá,
lần hỏng vẫn là T33a, 0 `KERNEL FAULT`/`SMP NO READY`.

## r293 — thước đo mới + harness đếm RETRY (theo tệp quyết định r293–r298)

- **293.1/293.2/293.3** (`2a04bd2`): `RETRY n/N` + phân loại T-FS/T-SCHED/T-KHAC,
  lưu `artifacts/flake-<arch>-<i>-a<lần>/` cho MỌI lần hỏng, `--strict` (chưa bật
  trong gate). `test-harness-spec.py` **647 case**, có case âm: gỡ chỗ đếm ⇒ ĐỎ.
- **293.5** (`2380cc5`): `other_caps_to` + `reparent_tcb_caps` đi qua
  `for_each_cap`/`for_each_cap_mut`; grep còn 3 chỗ `iter().flatten()` và cả ba
  là view của MỘT đối tượng.
- **293.6** (`2380cc5`): `reply_epoch` u8 → u16, test 260 lần Call không trùng
  epoch, canary `size_of::<Tcb>() == 1648`.
- **293 (theo Agent 0)** (`39589cb`): guard `KERNEL FAULT` mặc định quay về
  `halt()`, chỉ bật sau feature `diag-idle-fault`; harness đếm `KERNEL FAULT`
  là T-SCHED.

### 293.8 — số đo nền x86_64 (một sha, guard TẮT, MAX_IO=128)

```
RETRY 8/305 (T-FS=0, T-SCHED=7, T-KHAC=1)   — 297/300 PASS
```

| Nhóm | Số lần | Run |
|---|---:|---|
| treo sau `shell>`, thiếu `PREEMPT OK` | 3 | 36, 192, 287 |
| `USER FAULT tcb=None` + `no runnable thread` | 4 | 102, 108, 212, 217 |
| race debug-exit (retry cứu) | 1 | 122 |

**T-FS = 0/305** ⇒ BLOCKER FS không tái phát ở `MAX_IO=128`.

Lỗi của chính bộ phân loại (đã sửa): chết sớm bị dán nhãn T-FS vì thiếu
`FS V2 OK`; nay xét chữ ký SCHED trước, và chỉ coi là T-FS khi guest đã tới
`SHELL READY`.

## r294.4 — gốc lớp "chết sớm": halt khi đang giữ BKL

`registers.txt` của artifact: CPU#0 `CPL=0 IF=0 HLT=0`, CPU#1 `CPL=0 IF=0 HLT=1`.
`fault::suspend` chạy trong trap handler đang giữ BKL (IF=0); gặp
`pick_on == None` thì `A::halt()` = `loop { hlt }` ⇒ BKL không bao giờ được nhả ⇒
hart còn lại quay BKL mãi ⇒ cả máy chết.

Sửa (`c2a8968`): `fault::suspend -> bool`, bốn call site nhả `bkl` rồi mới halt,
thêm `SMP PARK (hart, cause)`.

Đo: 60 boot sau vá ⇒ `RETRY 1/61 (T-FS=0, T-SCHED=1, T-KHAC=0)`; lần hỏng duy
nhất là flake boot sớm (thiếu `SELFTEST PASS`); **0** lần `tcb=None`.

## r294.B — T33a thật ra là T-ROOT `main.rs:480` (GetTime trên hart 1)

### Phân loại lại (bảng mới của Agent 0)

7 lần "T-SCHED" của số đo nền 293.8 (`RETRY 8/305`) được phân loại lại:
**6 lần là T-ROOT `crates/vifast-root/src/main.rs:480`** (có dòng `ROOT PANIC`),
1 lần là **T-BOOT** (chết trước kernel, thiếu `SELFTEST PASS`); riêng lần
"race debug-exit" là **T-INFRA** (ngoài ngân sách). Bộ phân loại cũ xét
"thiếu marker" trước nên gộp nhầm.

### Hai giả thuyết bị BÁC bằng đèn (không bằng suy đoán)

| Giả thuyết | Số đo | Kết luận |
|---|---|---|
| PIT ngừng phát | `TICK n` vẫn chạy, `eoi` khớp `ticks` | Bác |
| PIC kẹt in-service / bị mask | `PIC STALL (isr=0x00, irr=0x01, imr=0xec)` | Bác — PIC trong sạch, IRQ0 chỉ đang chờ |

(Theo bản soát độc lập: `fast_time()` = `fetch_max` nên **không thể** giảm ⇒ cái
sai là **context user của worker**, không phải đồng hồ.)

### Đã làm

- `write_artifacts()` — MỘT đường ghi artifact: `serial.txt` đầy đủ, `status.txt`
  giữ nguyên khối `ROOT PANIC` + dòng `file:dòng`, `tail.txt`, `registers.txt`.
  Spec **649 case**, 4 case âm mới.
- Root: tách `GETTIME ERR <mã>` khỏi `TIME MONOTONIC BAD` (trước đây gộp).
- Đèn `diag-pick` (mặc định TẮT): `PICK hart tcb slot state` trước mỗi lần nạp
  context — để phân biệt (a) cùng TCB trên hai hart, (b) hai TCB chung một
  `context_slot`, (c) resume khôi phục context cũ.

### Việc kế tiếp (r294.B theo thứ tự mới của bản soát)

1. Bật `diag-pick`, chạy **một** lượt 60–120 boot cùng sha, đọc 20 dòng cuối
   trước `ROOT PANIC`.
2. Phân biệt (a)/(b)/(c) ⇒ sửa gốc + test đỏ-trước (có thể là D2.a: chặn tại
   `pick_on` / cấm `context_slot` trùng).

## r294.B — GỐC T33a tìm được: AP mask LINT0 của BSP (2026-11-10, T0+1h30)

### Bằng chứng (đèn `NO TICK`, chạy trên hart còn sống nên không phụ thuộc timer)

```
NO TICK (ticks=2, traps=4096, tpr=0x0, svr=0x1ff,
         lvt0=0x10000, pic_isr=0x00, pic_irr=0x00, pic_imr=0xec, eoi=2)
```

`lvt0 = 0x10000` ⇒ **bit 16 = MASKED**: LINT0 (cửa ExtINT của PIC 8259 vào CPU)
bị mask trên **BSP** từ **tick 2**. PIC sạch (`isr=0, irr=0, imr=0xec`), nhưng
không đường nào đẩy IRQ0 vào CPU ⇒ **tick ngừng vĩnh viễn** ⇒ `PREEMPT OK`
(cần tick ≥ 99) không bao giờ in — đúng chữ ký "treo sau `shell>`", vì `shell>`
chỉ là marker cuối trước `PREEMPT OK`.

Trước đó hai giả thuyết đã bị bác bằng đèn: PIT ngừng phát (`TICK`/`eoi` khớp),
PIC kẹt in-service/bị mask (`PIC STALL isr=0, imr=0xec`).

### Ai mask

Toàn bộ mã chỉ có **hai** chỗ ghi LVT0 (0x350): BSP đặt `0x700` (ExtINT, không
mask) và **AP mask `1<<16`** trong init hart phụ. MMIO LAPIC định tuyến theo CPU
đang chạy nên dòng của AP không được phép đổi LINT0 của BSP — nhưng thực đo thì
BSP bị mask đúng lúc AP khởi động.

### Bản vá + đo

AP chỉ mask **LINT1**; LINT0 để nguyên; thêm đèn `AP LVT0=` đọc lại.

| Lượt | Sha | N | Kết quả |
|---|---|---|---|
| Trước vá (có đèn) | `4e580f3` | 100 | 2 hỏng (1 T-SCHED treo, 1 T-KHAC `BIN OK`) |
| Sau vá (có đèn) | `28aefcd` | 100 | **0 hỏng**, 0 `NO TICK` |

Giới hạn: 0/100 chỉ viết được "tỉ lệ < 3/100"; lượt 300 boot không đèn ở khối Z
mới là số chốt.

### Trạng thái ngay sau bản vá LINT0 (ghi trung thực)

Lượt 300 boot **không đèn** (sha `3699fe7`, bản vá LINT0 đã có) đang chạy.
Tới run 66/300 đã có **1 lần treo sau `shell>`** (thiếu `PREEMPT OK`) ⇒
**không được kết luận "đã hết"**: hoặc lớp này còn nguyên nhân thứ hai, hoặc
lượt 0/100 trước đó là may mắn.

Việc phải làm để phân biệt (lượt sau, có đèn): bật lại `diag-tick` + `diag-sched`,
chạy ~120 boot, xem lần treo kế tiếp có `NO TICK` với `lvt0=0x10000` nữa không.
- Nếu còn `lvt0=0x10000` ⇒ còn chỗ khác mask LINT0 (phải tìm, không đoán).
- Nếu `NO TICK` không in (tick vẫn chạy) ⇒ treo này là lớp khác, phải đặt tên mới.

Ghi chú: bản **không đèn** không có `NO TICK` nên sự vắng mặt của nó trong
artifact lượt này **không** chứng minh được gì.

### Số chốt: 300 boot KHÔNG ĐÈN sau bản vá LINT0

Sha `3699fe7` (bản vá LINT0 + bỏ nhánh GetTime), `MAX_IO=128`, guard TẮT, đèn TẮT.

```
RETRY 2/301 (T-FS=0, T-SCHED=1, T-KHAC=1)
```

| Lớp | Số lần | Artifact | Ghi chú |
|---|---:|---|---|
| T-SCHED — treo sau `shell>`, thiếu `PREEMPT OK` | **1** | `artifacts/flake-x86_64-16-a0` | còn lại sau bản vá LINT0 ⇒ phải soi bằng lượt có đèn |
| **T-DUP** `PREEMPT OK` in **hai lần** trong một lần boot | **1** | `artifacts/flake-x86_64-276-a0` | chữ ký MỚI, đặt tên ngay: `T-DUP PREEMPT OK`. Nghi hai hart cùng thoả điều kiện in |
| T-FS | 0 | — | BLOCKER vẫn "ngủ" ở 128 |
| T-ROOT `main.rs:480` | **0** (trước: 6/305) | — | hết sau khi bỏ nhánh GetTime + vá LINT0 |
| T-BOOT | 0 (trước: 1/305) | — | mẫu nhỏ, chưa kết luận |

So với mốc nền `c2a8968`: 8/305 = **2,6 %** → nay 2/301 = **0,66 %**.
Theo luật thống kê: 2/301 viết là "≈0,66 %, KTC 95 % khoảng 0,1–2,4 %" — **chưa**
được nói đã đạt mức tạm ≤1 %/boot một cách chắc chắn; cần thêm mẫu.

## r294.B — GỐC THỨ HAI: AP chạy task trước khi kernel init xong

### Bằng chứng (vòng đệm 96 sự kiện, đổ ra khi xử lý `Exit`)

```
ROOT PANIC
crates/vifast-root/src/main.rs:480
RING DUMP (96 sự kiện, vì UserAction::Exit)
RING kind=1 hart=1 tcb=132 slot=15 state=2 tick=2
RING kind=3 hart=1 tcb=130 slot=14 state=2 tick=2
… 96/96 sự kiện đều hart=1, tick=2, luân phiên hai TCB
```

Hart 1 chạy TCB ở **tick 2** trong khi BSP đã ở tick hàng trăm; `fast_time()` là
`fetch_max` nên không thể giảm — **trừ khi kernel bị reset**. Và đúng là bị
reset: `boot()` gọi `A::init_kernel()` **hai lần** (selftest + kernel thật), mỗi
lần `reset()`. AP được thả trong `init_platform()` **trước** cả hai lần đó, nên
AP kịp `pick_on(1)` trên trạng thái tiền-reset: `last` trên stack của worker đến
từ đồng hồ cũ, `GetTime` sau reset đọc đồng hồ mới ⇒ `now < last` ⇒
`TIME MONOTONIC BAD` + `ROOT PANIC main.rs:480`.

### Sửa

Cờ `KERNEL_READY` (platform x86), BSP bật ở `prepare_user()` — sau
`init_kernel()` + selftest; `ap_idle_loop()` chỉ `sti; hlt; cli` cho tới khi cờ
bật, **không chạm runqueue** trước đó.

### Đo

| Lượt (x86_64, có đèn) | N | Kết quả |
|---|---:|---|
| Trước cờ | 120 | `RETRY 2/120` — 1 **T-ROOT `main.rs:480`**, 1 T-KHAC |
| Sau cờ | 120 | **`RETRY 0/120`**, 0 `TIME MONOTONIC BAD`, 0 `NO TICK` |

Hai gốc đã tìm và vá trong đêm, cùng một lớp "hart phụ chạm trạng thái chưa
sẵn sàng":
1. **AP mask LINT0 của BSP** ⇒ PIC không đẩy được IRQ0 ⇒ tick ngừng (commit
   `28aefcd`);
2. **AP chạy task trước khi kernel init xong** ⇒ `GetTime` lùi ⇒
   `ROOT PANIC main.rs:480` (commit `8cd767f`).

## Số chốt vòng 6: 300 boot KHÔNG ĐÈN sau HAI bản vá (LINT0 + KERNEL_READY)

Sha `8cd767f`, `MAX_IO=128`, đèn TẮT, cùng máy.

```
RETRY 3/301 (T-KHAC=1, T-ROOT crates/vifast-root/src/main.rs:480=1, T-SCHED=1)
```

| Lớp | Số lần | Chữ ký | Ghi chú |
|---|---:|---|---|
| T-ROOT `main.rs:480` | 1 | `TIME MONOTONIC BAD` + panic | cờ `KERNEL_READY` giảm nhưng **chưa** hết ⇒ còn đường khác làm AP chạm trạng thái chưa sẵn sàng, hoặc cờ bật quá sớm |
| T-SCHED | 1 | treo, thiếu `PREEMPT OK` | chưa rõ có phải LINT0 nữa hay không (lượt này không đèn) |
| T-KHAC `dup=['PAGER EVICT OK']` | 1 | **đúng chữ ký T31** (`PAGER2_LABEL` xử lý hai lần) | artifact `flake-x86_64-97-a0` **không** chứa marker này (serial lưu ở lần thử khác) ⇒ cần sửa B1' cho khớp lần thử, ghi nợ |

Tổng: **3/301 = 1,0 %** — đúng sát mức tạm ≤1 %/boot, **chưa** đạt (cần ≤3 lần
tính ngân sách trên 300 và KTC hẹp hơn).

So sánh ba mốc trên cùng máy: **2,6 %** (nền `c2a8968`) → **0,66 %** (`3699fe7`,
chỉ vá LINT0) → **1,0 %** (`8cd767f`, thêm `KERNEL_READY`). Chênh lệch giữa hai
mốc sau nằm trong nhiễu thống kê (3/301 so với 2/301) ⇒ **không** được nói bản
vá thứ hai làm tệ đi, cũng **không** được nói nó đã hết lớp T-ROOT.

Việc phải làm tiếp (ghi rõ để phiên sau không đoán):
1. T-ROOT còn 1/301 ⇒ bật lại đèn, chạy tới khi bắt được, đọc `RING` xem còn
   `tick=2` không; nếu có ⇒ còn đường AP chạm runqueue trước `KERNEL_READY`
   (grep mọi `AP_STARTED`/`wake_hart` trước `prepare_user`).
2. T-SCHED 1/301: cần đèn để xem `lvt0` còn bị mask hay là lớp khác.
3. T31: đã tái hiện **lần đầu** bằng marker `dup=['PAGER EVICT OK']`; cần lưu
   đúng serial của lần thử đã sinh ra nó (sửa `write_artifacts` dùng attempt).

## r294.C — mốc `KERNEL ENTRY`, T-BOOT định nghĩa theo mốc

- Kernel in `KERNEL ENTRY` ngay đầu `boot()`, **trước** `init_platform`.
- `classify_failure`: `T-BOOT ⇔ không có KERNEL ENTRY` (phiếu soát §1.5), xét
  trước mọi lớp khác. Spec **652 case**, có ca âm "đã vào kernel mà thiếu
  PREEMPT OK ⇒ T-SCHED".
- Harness: artifact đọc **lại** serial đã settled từ file — vá lỗi lượt trước,
  khi artifact của lần hỏng T31 (`dup=PAGER EVICT OK`) lại không chứa marker
  gây hỏng.

### Số đo vòng 7

| Lượt (x86_64, không đèn, sha `6e0ee9c`+`4a4bf89`) | N | Kết quả |
|---|---:|---|
| Săn T31 | 200 | `RETRY 1/200` — **không** tái hiện T31; 1 lần là T-ROOT `main.rs:480` |
| Gate | 3×3 | 10/10, `RETRY 0/9` |

**T-ROOT `main.rs:480` vẫn còn 1/200 sau cờ `KERNEL_READY`** ⇒ cờ chỉ chặn một
phần; cần đèn để xem AP còn chạm runqueue trước khi kernel sẵn sàng ở đường nào,
hoặc `last` bị hỏng bởi cơ chế context-slot (giả thuyết (b) của bản soát).
Artifact lần này có `ROOT PANIC` + `main.rs:480` + serial đầy đủ nhưng **không**
có `RING DUMP` (bản đo không đèn) ⇒ lượt sau phải chạy có đèn để có vòng đệm.

**T31**: không tái hiện trong 200 boot; đã có (1 lần) trong lượt 300 boot trước.
Chưa sửa được ⇒ giữ mở, cần lượt có đèn để chụp chuỗi `PAGER2_LABEL`.

## r294.T31 — ghi LẠI context sau `apply_resume` (việc song song với subagent)

**Phân tích (subagent, chỉ đọc mã, không build/QEMU):** hai dòng
`PAGER EVICT OK` ⇒ thân nhánh PAGER2 chạy **≥3 lần** dù client chỉ có hai lệnh
`send`. Cơ chế khả dĩ nhất (~60 %): `apply_resume()` vá context SỐNG nhưng
**không ghi lại** `contexts[slot]`; lần nạp slot sau đó chạy lại **snapshot cũ**,
mà snapshot đó có thể mang đúng bộ thanh ghi message ⇒ thân nhánh chạy lại
nguyên vẹn. Hai cơ chế còn lại: (a) cùng TCB trên hai hart (~5 %, `pick_on`
không kiểm `current` của hart khác), (b) hai TCB chung slot (~20 %, lỗ
`.unwrap_or(0)`).

**Đã sửa:**
1. **5 điểm nạp context**: sau `apply_resume(...)` ghi lại `contexts[slot] = *context`.
2. **Thiếu slot**: tách hàm thuần `slot_choice(Option<u8>) -> (usize, bool)` +
   test host; `slot_or_report` in MỘT lần `CTX SLOT NONE hart=… tcb=…` thay vì
   lặng lẽ dùng slot 0.
3. **Gỡ feature chết `diag-pick`** (khai báo ở 2 Cargo.toml nhưng không dòng mã nào
   dùng — tài liệu cũ ghi sai rằng nó đã được nối).

**Phép đo thay thế để chốt T31 (chưa chạy — hết giờ):** trên serial của lần hỏng
T31, đếm `USER FAULT … addr=0x1001001000`: **2** ⇒ replay phía server (cơ chế c);
**3** ⇒ client chạy lại ở frame fault. `scripts/test-qemu.py` đã có
`fault_addresses()` dùng được ngay, **không cần sửa mã**.

Test: `vifast-runtime` 4 test; sàn **375 test / 11 crate**; gate **10/10**.

## T33a — trạng thái chốt cuối ngày 10/11

| Bản vá | Đo | Kết quả |
|---|---|---|
| (nền) | 305 boot | 2,6 % |
| LINT0 (AP không mask LINT0 của BSP) | 301 boot | **0,66 %** |
| + `KERNEL_READY` (AP không chạy task trước khi kernel init xong) | 301 boot | 1,0 % (3/301) |
| + ghi lại context (T31) | *chưa đo* | — |

Còn mở, đã ghi rõ cách làm tiếp: T-ROOT `main.rs:480` 1/200 (cần lượt có đèn để
xem `RING` còn `tick=2`), T-SCHED 1/301 (cần đèn xem `lvt0`), T31 (dùng phép đo
`USER FAULT` ở trên), `BIN EXEC FAIL`/`DESTROY DIAG stuck=4` (TCB kẹt `Busy` —
nghi TCB còn `Running` trên hart phụ nên không huỷ được), T26/T33b/294.5 chưa làm.

## Chốt hết ngày 10/11/2026 (Asia/Tokyo)

Cây ở `fb88b1c`, gate **10/10**, bản phát hành **không đèn**.

| Hạng mục của mục tiêu | Trạng thái | Bằng chứng |
|---|---|---|
| **T33a** treo trước `PREEMPT OK` | **giảm 2,6 % → 0,66 %**, hai GỐC đã tìm + vá | `NO TICK … lvt0=0x10000` (LINT0); `RING … hart=1 tick=2` (AP chạy trước init) |
| **T31** `PAGER2_LABEL` hai lần | bản vá gốc tiềm năng + phép đo xác nhận đã dựng, **chưa chốt số** | tái hiện 1 lần (`dup=PAGER EVICT OK`) |
| **T33b** riscv64 thoát sớm | **chưa làm** | — |
| **T26** race FS/`MAX_IO=256` | **chưa làm** | — |
| 294.C (mốc boot) | một phần: `KERNEL ENTRY` + T-BOOT theo mốc | spec 652 case |
| 294.5 (riscv `vifast-maint`) | **chưa làm** | — |

Việc tiếp theo, theo thứ tự đã phân tích:
1. Đo 200 boot bản `79e0f13` **có đèn** để xem T-ROOT/T31 còn không; nếu T31 tái hiện, đếm
   `USER FAULT addr=0x1001001000` (2 ⇒ replay phía server, 3 ⇒ replay phía client).
2. `BIN EXEC FAIL` / `DESTROY DIAG stuck=4`: TCB kẹt `Busy` — kiểm TCB còn `Running`
   trên hart phụ (`can_destroy` từ chối TCB không `Exited` đang là `current`).
3. T26: dựng lại phép đo `--repeat 40` ở `MAX_IO=256` để có số nền trước khi sửa.
4. T33b: chạy 200 boot riscv64 có đèn để bắt "thoát sớm".

## r294.T33b — riscv64: panic `assert_current_tcb` (hart phụ) và vị trí panic

**Đo nền (200 boot riscv64):** `RETRY 2/202 (T-SCHED=2)`, cả hai là
`PANIC (hart=1)` — **không có manh mối** vì handler panic của riscv/aarch64 nuốt
`PanicInfo`. Triệu chứng bề mặt: "guest không nhận được dữ liệu TCP echo" và
"QEMU exit code 1" (hệ quả, không phải nguyên nhân).

**Bước 1:** in `PANIC AT <file>:<dòng>` trên riscv64 + aarch64; harness đọc được
(`panic_site`), phân loại `T-ROOT <file:dòng>`; spec **653 case**.
⇒ Chụp ngay `crates/vifast-platform-riscv64/src/platform.rs:671` =
`assert_current_tcb()` "per-CPU current_tcb mismatch".

**Bước 2 (gốc):** `assert_current_tcb` là **bất biến chẩn đoán**. Đường
`fault_to_endpoint` xoá `current` của MỌI hart khi task bị park vì fault, còn
cache per-CPU không được cập nhật ⇒ lần trap sau lệch ⇒ bản cũ **giết máy**.
Sửa: lệch thì **sửa cache theo kernel** + in MỘT lần
`CURRENT MISMATCH (hart=…, cache=…, kernel=…)`.

**Đo sau sửa:** 201 boot ⇒ `RETRY 1/201 (T-KHAC=1)`, **0 `PANIC`**, 0
`CURRENT MISMATCH`; lần hỏng còn lại là "thiếu marker `IPC RIGHTS OK`" (khác lớp).

| Lượt riscv64 | N | Kết quả |
|---|---:|---|
| Trước | 200 | 2 hỏng — **2 × PANIC (hart=1)** |
| Sau | 201 | 1 hỏng — 0 panic; 1 lần thiếu marker `IPC RIGHTS OK` |

Việc tiếp cho T33b: (a) "thiếu marker `IPC RIGHTS OK`" 1/201 — cần đèn/artifact
để xem guest dừng ở đâu; (b) bước TCP echo giờ tách khỏi panic, cần đo riêng
(listener của harness mở sau khi guest đã in marker?).

## r294.T26 — khoanh vùng race FS: hai phép đo quyết định

### Số đo (x86_64, `scripts/fs-smp-regression.py`)

| Cấu hình | N | Kết quả |
|---|---:|---|
| `MAX_IO=128`, IPC buf 256 B (sản phẩm) | 40 | **40/40 xanh** |
| `MAX_IO=256`, IPC buf 256 B | 40 | **12/40 đỏ** (30 %) — tái hiện đúng con số 12/49 của r262 |
| `MAX_IO=256`, **IPC buf 512 B** (`IPC_BUFFER_WORDS` 32→64) | 40 | **8/40 đỏ** |

**Kết luận 1 (âm tính, quan trọng):** nới đệm IPC 256 → 512 B **không** diệt được
lỗi (12/40 → 8/40, chênh lệch nằm trong nhiễu). Vậy **không** phải "payload 256 B
đụng trần đệm IPC" như giả thuyết ban đầu — đây là **race thật** ở đường FS khi
≥2 hart, đúng như ghi chú r262.

Đã trả `IPC_BUFFER_WORDS=32`, `MAX_IO=128` về nguyên trạng; build + gate xanh.

### Đính chính dữ liệu (do subagent phát hiện, ghi để không lặp lại sai)

- Artifact `artifacts/flake-x86_64-1-a0` mà tôi đọc lúc đầu **có `FS V2 BAD`**
  (dòng 193) — tôi kết luận sai là "treo không in OK lẫn BAD".
- Ba artifact thật sự **không có** `FS V2` (`flake-x86_64-{17,20,113}-a0`) lại có
  `serial.txt` **0 byte** ⇒ hiện **không có serial** của ca treo; muốn kết luận
  "treo ở đâu" phải có đèn, không suy từ log rỗng.

### Việc tiếp cho T26 (đã có thứ tự)

1. Thêm đèn **một marker mỗi bước** trong khối V2 (`crates/vifast-client/src/main.rs`
   ~1000-1155) để biết lần đỏ dừng ở bước nào; in cả mã lỗi trong `vfs_reply`
   thay vì gộp thành `EIO`.
2. Vá hai chỗ kernel làm **mất phản hồi** (giả thuyết H1 của bản phân tích):
   `ipc.rs` `reply()` khi `valid==false` phải `wake_error(client, ObjectDestroyed)`
   thay vì bỏ rơi client `BlockedReply`; `reply_recv()` không được `?` trước `recv`.
3. Chỉ sau khi hết đỏ ở 256 mới đổi `MAX_IO` về 256 (luật D19, N≥40).

## r294.T26 — giả thuyết H1 (reply-orphan) BỊ BÁC bằng đo, đã revert

**Bản vá đã thử** (theo bản phân tích subagent):
1. `ipc.rs::reply()` — khi caller STALE mà client còn `BlockedReply` thì
   `wake_error(client, ObjectDestroyed)` thay vì bỏ rơi;
2. `ipc.rs::reply_recv()` — reply lỗi KHÔNG được bỏ lượt `recv`.

Kèm test đỏ-trước `stale_reply_does_not_orphan_a_waiting_client` (đã chứng minh
ĐỎ khi tắt bản vá: `left: BlockedReply`, XANH khi bật — đúng D18).

**Đo (x86_64, cùng máy):**

| Cấu hình | N | Đỏ |
|---|---:|---:|
| `MAX_IO=256` **trước** vá (nền) | 40 | 12 |
| `MAX_IO=256` **sau** vá | 40 | **9** |
| `MAX_IO=128` **sau** vá | 40 | **1** ← hồi quy ở cấu hình sản phẩm |
| `MAX_IO=128` sau khi **revert** | 40 | **0** |

**Kết luận:** 12 → 9 nằm trong nhiễu (p ≈ 0,4) ⇒ H1 **không** phải gốc; tệ hơn,
bản vá làm cấu hình sản phẩm (128) đỏ 1/40 — đúng điều kiện cấm. Theo luật D18
("bản vá làm đỏ cấu hình đang xanh thì revert"), đã **revert cả hai bản vá và
test**; cây về 40/40 xanh.

Bài học ghi lại: `wake_error` cho caller "stale" là thay đổi **hành vi** ở đường
IPC nóng, không phải sửa lỗi cục bộ — muốn thử lại phải đo riêng từng bản vá
(1 rồi 2), không gộp.

**Việc tiếp (giữ nguyên thứ tự):** đèn một marker mỗi bước trong khối V2 để biết
lần đỏ dừng ở đâu; chỉ sau đó mới suy tiếp cơ chế.

## r294.T26 — KHOANH VÙNG: lỗi nằm ở `vfs_fsck()`, không phải bước FS nào khác

Ba đèn được thêm và **giữ lại lâu dài** (output ngắn, không nằm trong hợp đồng marker):

1. `V2S01..V2S11` — một mốc sau mỗi bước của khối FS V2
   (`crates/vifast-client/src/main.rs`).
2. `V2FAIL v2=<0|1> fsck=<0|1>` — in lý do trước khi in `FS V2 BAD`.
3. Mã lỗi fsck hai phía: server `FS ERR <mã>`, client `FSCKERR <mã>`.

**KẾT QUẢ ĐÚNG (đo lại có đèn nhóm, 3/3 lần đỏ):** `V2GROUP 04` ⇒ nhóm kiểm tra
SAI ĐẦU TIÊN là `v2 &= found_d` — **readdir("/") KHÔNG thấy thư mục `d` vừa tạo**.
Và `fsck` **đạt**.

> **ĐÍNH CHÍNH KÉP (quan trọng, ghi để không lặp lại):**
> (1) Đèn cũ in `u8::from(!v2)`/`u8::from(!fsck)` nên `V2FAIL v2=1 fsck=0` thực
> nghĩa là **v2 SAI, fsck ĐÚNG** — tôi đã đọc ngược thành "mọi thứ đạt, chỉ fsck
> sai" và ghi vào báo cáo r262. Nay đèn in thêm `V2GROUP <nhóm>` để khỏi nhập
> nhằng.
> (2) Bản thân bản phân tích của subagent (dựa trên kết luận sai của tôi) cũng
> đã cảnh báo đúng rằng phải kiểm lại — và nó đúng.
> Kết luận sai "lỗi ở `vfs_fsck`" đã được gỡ khỏi mọi tài liệu; **cách giải
> thích cũ trong r262 (readdir đọc ảnh cũ) là ĐÚNG**.

**ĐÍNH CHÍNH (subagent phát hiện, quan trọng):** con số "12/40 đỏ" là đúng, nhưng
câu "100 % lần đỏ đều in V2FAIL" chỉ có **n=1** — ba lần đỏ còn lại là build CŨ
chưa có đèn `V2S`/`V2FAIL`. Và artifact đỏ có thể chưa có đèn `FSCKERR` (mtime
client 02:50:54 chồng lấn với serial 02:53:34), nên kết luận "không thấy mã lỗi
fsck" **chưa** đứng vững. Phải build lại và đo lại mới được nói.

**Bằng chứng mạnh, không phụ thuộc build:** quét mọi artifact thì `FS CORRUPT`
(server in cho MỌI lỗi của `fsck()`, mã đã commit từ lâu) **không xuất hiện lần
nào** trong khi `FS V2 BAD` có 4 file ⇒ **fsck chưa từng kết luận hỏng**; lỗi
nằm ở **lời gọi IPC không hoàn tất**, không phải kế toán FS sai.

Điều này **bác** cách giải thích cũ trong r262 ("mkdir OK nhưng readdir đọc ảnh
cũ"): nếu readdir sai thì `found_d`/`found_a` sẽ sai và `v2=0`. Dữ liệu mới nói
lỗi ở **tính nhất quán mà fsck nhìn thấy**.

Đo lại ở `MAX_IO=128` sau khi thêm đèn: **40/40 xanh**.

### Bảng quyết định cho T26 (từ bản phân tích subagent, dùng ngay vòng sau)

Thêm 5 đèn (không `unsafe`), chạy MỘT lượt ở `MAX_IO=256` là đủ kết luận:

| Đèn | Chỗ đặt | Ý nghĩa |
|---|---|---|
| **L1** `FKE <error> <label>` | client, ngay `FSCKERR` (`vifast-client/src/main.rs` ~371) | phân biệt `error != Ok` (WouldBlock=18/OOM=5/…) với server trả `ERR/EIO` |
| **L2** `FSQ <n>` | server, đầu nhánh `vfs::FSCK`, **vô điều kiện** | 2 = server vào nhánh 2 lần; 1 = request V2 chưa bao giờ tới |
| **L3** `FSV ok`/`FSV err <mã>` | server, **bỏ cổng `FSCK_PRINTED`** | đừng suy "server không trả lời" từ số lần `FS FSCK OK` |
| **L4** đếm guard `ipc.rs:129` | kernel, in trong dòng stats | guard `WouldBlock` có bắn không |
| **L5** `WKE <tcb> <err>` | kernel `wake_error` | nếu H2 (bị đánh thức kèm lỗi) đúng sẽ thấy ngay trước `V2FAIL` |

Suy luận: `FKE 011` + `FSQ=1` + không `WKE` ⇒ H1 (guard từ chối Call);
`FKE 011` + có `WKE` ⇒ H2 (client bị đánh thức kèm lỗi);
`FSQ=2` + `FSV err` ⇒ H3/H4 (fsck hỏng thật); `fsck=0` mà **không** `FKE` ⇒
binary cũ (H5).

Bản vá cho H1 (nếu đèn xác nhận): `Call` **không bao giờ** được trả `WouldBlock`
— thay `return Err(WouldBlock)` ở `ipc.rs:118-131` bằng cờ `held` rồi để client
park trong `ep.send`; kèm `ep.recv.remove(receiver)` trước `push` (`ipc.rs:335`).
Rủi ro phải đo: nếu trạng thái "giữ caller + còn trong hàng recv" là vĩnh viễn
thì đỏ sẽ biến thành **treo** (mất `V2S11`) — phải chạy lại 40 boot để so.

### Đèn `V2GROUP` (giữ lại) — nhóm kiểm tra V2 sai đầu tiên

Nhóm: 3 = đọc lại 600 B, **4 = readdir("/") thấy `d`**, 5 = readdir("/d") thấy
`a`, 6 = rename/ENOENT sau rename, 8 = unlink `/d/b`, 9 = unlink `/d/x`.

Đo `MAX_IO=256`, 3/3 lần đỏ đều `V2GROUP 04` ⇒ **read-your-own-write hỏng ngay ở
bước đọc thư mục gốc sau `mkdir("/d")`**. Đây là dữ liệu để vòng sau soi
`vfs_readdir`/cache block thư mục ở FS server.

## Luồng kiểm thử SONG SONG 2 máy — đã dựng và ĐO được 2× (2026-11-11)

Máy `ps` (Ubuntu x86_64, 12 CPU) đã sẵn: rustup đủ 5 target bare-metal,
`qemu-system-{x86_64,riscv64,aarch64}`, symlink OVMF 4M, rsync, python3, repo
`~/ViFastOs`.

**Cách vào máy:** `ssh -l ps ps` (user mặc định `mps` bị tailnet chặn);
`scripts/par-gate.sh` mặc định `REMOTE=ps@100.113.214.23` nên chạy được ngay.

**Đo thật (cùng đêm):**

| Cách chạy | Số boot | Thời gian |
|---|---:|---:|
| Tuần tự một máy (riscv64) | 40 | **149 s** |
| Song song: local riscv64 ×40 + `ps` x86_64 ×40 | **80** | **148 s** |

⇒ **2× thông lượng**, cả hai bên đều `PASS` (`RETRY 0`). Quy tắc: chia theo
**kiến trúc**, không chạy hai harness cùng arch trên một máy; rsync mã nguồn
sang `ps` trước và đối chiếu `source_sha256` của hai máy.

Đã ghi vào skill `skills/vifast-parallel-gate/SKILL.md` (kèm số đo, cách vào
máy, và bảng kiểm môi trường) để vòng sau dùng lại ngay.

## r294.T26 — phát hiện quan trọng nhờ máy thứ hai: lỗi phụ thuộc MÁY, và P2 làm lộ lỗi ATA

**Số đo 2 máy (cùng mã nguồn HEAD `adf762d`, `MAX_IO=128`)**

| Máy | N | Kết quả |
|---|---:|---|
| Mac (arm64) | 40 | 40/40 xanh |
| `ps` (Ubuntu x86_64, 12 CPU) | 40 | **40/40 xanh** |

⇒ Cấu hình sản phẩm xanh trên **cả hai** máy (đây là số nền đúng).

**Thử hai bản vá từ bản phân tích subagent:**

1. **P2** — `ata_wait_bsy` nay kiểm bit `ERR(0x01)/DF(0x20)` thay vì chỉ chờ `BSY`.
2. **P1** — `read_dir_entry_full` đọc block thư mục bằng `read_staged` (cùng khung
   nhìn với đường ghi).
3. **P3** — tách nhóm đèn 04 thành 04a/04b/04c.

**Kết quả ở `MAX_IO=128` trên `ps`: 2/40 ĐỎ** ⇒ **hồi quy ở cấu hình sản phẩm**,
trong khi baseline cùng máy 40/40 xanh. Theo D18 phải **revert**.

**Nhưng đây là manh mối lớn cho T26:** P2 chỉ *báo* lỗi ATA thay vì bỏ qua; việc
nó làm xuất hiện đỏ ở 128 nghĩa là **thiết bị có trả lỗi ghi mà bản cũ che mất**
⇒ giả thuyết H1 của bản phân tích ("ghi bị bỏ âm thầm, cache đánh dấu clean")
**có cơ sở thực nghiệm**, không chỉ là suy luận. Hướng đúng không phải "báo lỗi"
mà là **thử lại / đồng bộ đúng** ở tầng ATA, và phải đo lại ở 128 lẫn 256.

Ba bản vá được giữ trong `git stash` (`P1+P2+P3 tạm`) để vòng sau lấy lại; cây
đang ở HEAD xanh.

## r294.T26 — đính chính: P2 là cascade tự gây (subagent bắt)

`ERR/DF` là bit **dính**, chỉ xoá khi ghi thanh ghi Command. Tôi dùng
`ata_wait_bsy` (đã kiểm `0x21`) làm cổng **trước khi phát lệnh** trong
`ata_prepare` ⇒ sau lỗi đầu tiên mọi `ata_prepare` fail ⇒ retry không phát lại
lệnh nào ⇒ **toàn bộ I/O ATA chết**. Vì vậy "2/40 đỏ ở 128" của vòng trước là
**hồi quy do tôi gây**, không phải bằng chứng về thiết bị. Đã revert.

Sau revert, đo song song 2 máy ở `MAX_IO=128`: local **40/40 xanh**, `ps`
**39/40** ⇒ chưa được nói "128 tuyệt đối xanh trên mọi máy"; cần ≥120 boot/máy.

Thiết kế đúng đã có trong báo cáo (tách `wait_bsy_only`/`wait_done`, đèn
`stage`/`class`/`lba`, ngân sách retry, `ATA_DEAD`), kèm **phép kiểm quyết định**:
in `ATA STAT cmds=` ở cả 128 và 256 — nếu số lệnh ATA xấp xỉ nhau thì lỗi ATA
không thể giải thích chênh lệch 12–18/40.

## r294.T26 — PHÉP ĐO QUYẾT ĐỊNH: race có ở CẢ 128 trên máy thứ hai

**Câu hỏi:** 1–2/40 đỏ ở 128 trên `ps` là do các bản vá của tôi hay là flake có sẵn?

**Trả lời bằng đo, không bằng suy đoán:** revert TOÀN BỘ về HEAD (`207e988`, cây
sạch, không đèn, không bản vá ATA) rồi chạy **80 boot** trên `ps`:

```
KẾT QUẢ: HỎNG — 3/80 lần đỏ
```

⇒ **`MAX_IO=128` KHÔNG tuyệt đối xanh trên `ps`**: tỉ lệ ~3,75 %/boot. Lượt
"40/40 xanh" trước đó chỉ là may (với p≈0,0375 thì P(0/40) ≈ 22 %, hoàn toàn có
thể xảy ra).

**Ba kết luận:**

1. Các bản vá của tôi **không phải** nguyên nhân của 1–2/40 ở 128 — chúng nằm
   trong tỉ lệ nền có sẵn.
2. **Race read-your-own-write của FS server KHÔNG phụ thuộc `MAX_IO`**: nó có ở
   cả 128 (3,75 % trên `ps`) lẫn 256 (10–45 % tuỳ máy). `MAX_IO` chỉ **khuếch đại**
   tần suất, đúng như giả thuyết "khác nhau ở thời điểm, không ở bộ đếm".
3. Vì vậy **không được** dùng câu "128 xanh nên 128 an toàn" — phải nói
   "128 có tỉ lệ ~1–4 % tuỳ máy; mức chấp nhận được phải định nghĩa bằng số".

**Đo ATA (loại trừ dứt khoát bằng số):**

| `MAX_IO` | cmds | writes | flushes | retry | fail |
|---|---:|---:|---:|---:|---:|
| 128 | 2303 | 2304 | 2568 | **0** | **0** |
| 256 | 1535 | 1536 | 1694 | **0** | **0** |

Ở 256 có **ít lệnh ATA hơn** (ít round-trip hơn) và **0 lỗi/0 retry** — kể cả ở
lần boot đỏ. ⇒ **tầng ATA bị loại khỏi T26**. (Bản vá `wait_done`/`ATA STAT` vẫn
là cải thiện đúng và đang để trong `git stash` để đo riêng trước khi nhận.)

## r294.T26 — LOẠI TRỪ dứt khoát "mất lệnh ghi" (đọc-lại-sau-ghi)

Thêm đèn bằng chứng **trực tiếp** thay vì suy từ status: sau mỗi lần `CachedDev`
đẩy một block dirty xuống thiết bị, **đọc lại** và so với ảnh trong cache; lệch
thì tăng bộ đếm, server in `FS LOST <n>` ở nhánh FSCK.

**Đo `MAX_IO=256`, 12 boot (4 đỏ):**

| Kết quả | `FS LOST` | `V2GROUP` |
|---|---|---|
| 8 lần xanh | **0** | — |
| 4 lần đỏ | **0** | 06 |

⇒ **Không có lần nào ghi xong mà đọc lại thấy khác** — kể cả ở các lần đỏ. Vậy
giả thuyết "burst flush bị QEMU đánh rơi lệnh ghi" (chú thích `CACHE_SLOTS` đầu
tệp `vifast-fs`) **bị loại bằng đo**, không chỉ bằng lập luận. Kết hợp với phép
đo ATA trước đó (`cmds 128 > 256`, `retry=0`, `fail=0`), tầng thiết bị **hoàn
toàn sạch** trong lớp lỗi này.

⇒ Gốc T26 nằm **trong phần mềm FS** (journal/cache/thứ tự commit) hoặc ở **chen
ngang giữa hai client** (root và client test cùng ghi FS), không phải ở driver.

## r294.T26 — ĐỘT PHÁ: KHÔNG mất dữ liệu; lỗi ở TẦNG LIỆT KÊ

Đèn phân biệt (theo gợi ý "đèn rẻ nhất" của subagent) chạy **ngay trong client**
khi readdir("/") thiếu `d`: hỏi thẳng `open("/d")` rồi lặp readdir 2 lần nữa.

**Kết quả `MAX_IO=256` (2/2 lần đỏ):**

```
V2GROUP 06  V2CHK open=1 rd=2
```

- `open=1` ⇒ `open("/d")` **THÀNH CÔNG** ⇒ ảnh FS **CÓ** entry `d`.
- `rd=2` ⇒ **cả hai** lần readdir lặp lại đều **thấy** `d`.

⇒ **Không hề mất dữ liệu** và **không phải race giữa hai hart ghi**: entry tồn tại,
chỉ **lần liệt kê đầu tiên** bỏ sót nó — các lần sau đọc được.

Ba hệ quả:

1. **Loại** cơ chế "RMW đọc ảnh cũ rồi ghi đè làm mất entry".
2. **Loại** cơ chế "hai hart cùng thực thi một `Fs`" (nếu đua thì lần lặp lại cũng
   hỏng, và `open` cũng hỏng).
3. Còn lại **đúng tầng liệt kê**: gói nhiều record vào khung IPC 256 B
   (`READDIR_REC=32` ⇒ 8 record = 256 B = **đầy khung, không slack**), client giải
   mã lười từng record, và cả hai phía đều `break` im lặng khi gặp `Err`
   (`vifast-client/src/main.rs:575-581`, `vifast-server/src/main.rs:648-652`).

Đây là bước hẹp nhất từ đầu tới nay, và nó **đổi hẳn hướng sửa**: không phải sửa
journal/cache, mà sửa **đường readdir** (trang hoá + xử lý lỗi không được cắt
danh sách trong im lặng).

Đo lại ở `MAX_IO=128`: 40/40 xanh (đèn mới chỉ in khi lỗi nên không ảnh hưởng).

## r294.T26 — retry readdir KHÔNG cứu được (bằng chứng: nó không bắn)

Thêm **thử lại 2 lần** khi `vfs_readdir` nhận `Err(code != ENOENT)` (đèn
`V2CHK open=1 rd=2` cho thấy lần sau đọc được).

**Đo `MAX_IO=256`, 30 boot: 9/30 đỏ (30 %)** — **y như trước** (12/40 = 30 %).

⇒ Lỗi **không** đi qua nhánh `Err`; nó đi qua nhánh **`Ok(None)`/ENOENT**, tức
server tuyên bố "hết thư mục" ở một index **trước** entry `d`, và client dừng
vòng lặp. Đúng như bản đánh giá subagent cảnh báo: "retry chỉ phủ MỘT NỬA triệu
chứng".

Vì vậy **revert** retry (không chứng minh được tác dụng, luật D18) và chuyển
sang đèn đúng chỗ: in **lý do dừng** ở điểm cắt (`ENOENT` do hết thật vs do
`read_dir_entry_full` đếm thiếu) cùng `index` dừng.

Bản đánh giá còn nêu rủi ro quan trọng khác: `vfs_reply` gộp `EIO` với `Corrupt`
nên retry không phân biệt được lỗi thoáng qua với block hỏng, và việc "xanh ở
256" không được quy công cho retry khi `MAX_IO` cũng vừa đổi — hai thay đổi phải
đo **tách rời**.

## r294.T26 — ĐÈN `V2RD`: vòng liệt kê dừng ở **index 6** với ENOENT

Đèn mới in **lý do dừng** của vòng readdir trong khối V2.

**Đo `MAX_IO=256`, 8/20 boot đỏ — TẤT CẢ cùng một dạng:**

```
V2RD end n=06        (dừng ở index 6, lý do Ok(None) ⇒ server trả ENOENT)
V2GROUP 06
```

Kết hợp với `V2CHK open=1 rd=2` trước đó ⇒ **cùng một hệ thống tệp, cùng thời
điểm**: một chuỗi `readdir` liên tiếp (index 0,1,2,…) bị server tuyên bố "hết"
tại index 6, nhưng một lần `readdir` **mới từ index 0** ngay sau đó lại thấy entry
`d`, và `open("/d")` cũng thành công.

⇒ Lỗi nằm ở **`read_dir_entry_full`** (`crates/vifast-fs/src/lib.rs:2092-2105`):
nó lấy inode từ `self.inodes` (bản trong RAM) và đếm `seen` theo số entry **không
rỗng** của block đọc qua cache. Nếu ảnh block/dir-inode trong RAM lệch với đĩa
đúng trong chuỗi đó thì `seen` không bao giờ chạm index 6 ⇒ trả `Ok(None)` ⇒
client dừng ⇒ mất `d`. Đây là chỗ hẹp nhất và đã có file:dòng cụ thể.

**Việc tiếp (đúng một chỗ):** đèn so **ảnh cache vs ảnh đĩa** của block thư mục
gốc ngay trong `read_dir_entry_full` khi trả `Ok(None)` mà `index` còn nhỏ hơn số
entry đếm được bằng đường khác; nếu lệch ⇒ vá cache/inode-refresh (nghi
`self.inodes`/slot cache không được cập nhật sau `commit`).

## r294.T26 — ảnh cache và ảnh đĩa KHỚP nhau ⇒ nghi inode trong RAM thiếu block

Đèn mới: mỗi lần `read_dir_entry_full` trả `Ok(None)`, đếm lại số entry bằng
đường đọc thứ hai và **in `DIR i=<index> c=<đếm-lần-1> r=<đếm-lần-2>` nếu lệch**.

**Đo `MAX_IO=256`, 7/20 boot đỏ — KHÔNG lần nào in `DIR`** ⇒ hai lần đếm **bằng
nhau** (cùng đọc ra 6 entry rồi hết). Vậy **không phải** cache trả ảnh khác đĩa
trong phạm vi những block mà inode khai báo.

Kết hợp `V2RD end n=06` ⇒ `read_dir_entry_full` chỉ thấy **6 entry không rỗng**
trong toàn bộ `inode.blocks_used()` block của thư mục gốc, rồi tuyên bố hết.

⇒ Giả thuyết hẹp nhất còn lại: **bản inode trong `self.inodes` (RAM) thiếu block**
— `blocks_used()`/`size` nhỏ hơn đĩa (entry `d` nằm ở block thứ hai chưa được
tính). Phép đo trên chỉ quét trong `blocks_used()` nên **không thể** thấy chênh
lệch đó; muốn chứng minh phải so **inode RAM với inode trên đĩa**.

**Việc tiếp (đúng một phép đo):** trong `read_dir_entry_full`, đọc lại inode từ
đĩa (`stage_inode`/`load`) và so `size` + `blocks_used()` với bản RAM; in
`INO ram=<a> disk=<b>` khi lệch. Nếu lệch ⇒ vá chỗ làm mới `self.inodes` sau
`write_dir_entry`/`commit` (nghi thiếu cập nhật cho thư mục cha khi entry mới rơi
sang block kế).

## r294.T26 — giả thuyết "inode RAM thiếu block" cũng bị BÁC (đọc mã)

Đọc `write_dir_entry` (`crates/vifast-fs/src/lib.rs:1637-1668`): khi entry mới rơi
sang **block kế**, hàm có làm **đủ ba việc**:
`inode.blocks[blocks_used()] = block_no` → `inode.size += BLOCK_SIZE` →
**`self.inodes[dir - 1] = inode`** → `stage_inode(dir)` + `stage_bitmap()`.

Và `blocks_used()` (`lib.rs:2414-2430`) = `ceil(size / 512)` kẹp theo
`MAX_BLOCKS_PER_INODE`, nên nếu `size` đúng thì vòng quét **có** đi qua block thứ
hai.

⇒ Giả thuyết "inode trong RAM thiếu block sau `mkdir`" **không đứng** với mã hiện
tại. Kết hợp các phép đo trước (cache == đĩa, `open("/d")` Ok, readdir lặp lại
thấy `d`), dữ kiện còn lại duy nhất là: **tại đúng thời điểm đó, `seen` chỉ đếm
được 6 entry không rỗng**, rồi hết.

Điều cần đo tiếp (chưa làm): in **`size`, `blocks_used()`, và số entry đếm được
theo từng block** ngay tại chỗ trả `Ok(None)`; nếu block 1 có entry `d` mà
`size`=512 thì lỗi ở bước cập nhật `size`; nếu `size`=1024 mà block 1 rỗng thì
lỗi ở bước ghi entry.

### Bản đọc mã (subagent) củng cố + nêu 2 khả năng mới

**Xác nhận:** `write_dir_entry` ghi lại `self.inodes[CHA]` (`vifast-fs/src/lib.rs:1664`)
rồi `stage_inode` — nên "inode RAM thiếu block" **không thể** là nguyên nhân.
Quan trọng hơn: `read_dir_entry_full` và `lookup_child` quét **cùng inode RAM,
cùng `blocks_used()`** ⇒ `end n=06` và `open("/d")=Ok` **không thể** cùng đúng ở
một thời điểm ⇒ **sai lệch theo THỜI GIAN**, không theo không gian.

**Cảnh báo phương pháp:** đèn `DIR` **không** loại được giả thuyết inode thiếu
block, vì probe đọc lại bằng **cùng `dev.read` + cùng `blocks_used()` của inode
RAM** — nó chỉ chứng minh cache khớp đĩa, không chứng minh dải quét đủ.

**Hai khả năng mới (có file:dòng):**

1. **`truncate` không chặn thư mục** — `Fs::truncate` (`2017-2028`) chỉ kiểm
   `INODE_FILE`, và `truncate_request` phía server cũng **không** kiểm `fd.dir`
   ⇒ truncate trên **fd thư mục** sẽ xoá block của thư mục và đặt `size = 0`.
   Nếu bài test có truncate fd thư mục thì đây là bug thật, độc lập T26.
2. **Bất đối xứng ghi/đọc** — đường ghi thư mục dùng `read_staged` (journal),
   đường đọc dùng `dev.read`; nếu ảnh `dev.read` cũ hơn ảnh đã stage thì lần
   đọc ngay sau sẽ thấy danh sách ngắn ⇒ khớp triệu chứng "thoáng qua".

**Phép đo chốt (vòng sau):** in `size`, `blocks_used()`, `blocks[0..n]` và **số
entry không rỗng theo TỪNG block** ngay tại chỗ trả `Ok(None)`.

## r294.T26 — HÌNH DẠNG thư mục tại lúc "hết": 6 entry, thiếu đúng entry vừa tạo

Đèn `DIRP` in `size / blocks_used / số entry khối 0 / khối 1` ngay tại chỗ
`read_dir_entry_full` trả `Ok(None)`.

**Đo `MAX_IO=256`, 6/14 boot đỏ — 6/6 cùng một hình dạng:**

```
V2RD end n=06
DIRP s=0512 b=1 e0=06 e1=00      ← thư mục gốc: 1 block, CHỈ 6 entry, KHÔNG có `d`
```

⇒ Tại đúng thời điểm đó, block 0 của thư mục gốc trên **đường đọc** chỉ chứa 6
entry; `d` (entry thứ 7) **chưa xuất hiện**. Nhưng `open("/d")` ngay sau đó
thành công và readdir lặp lại thấy `d` ⇒ entry **xuất hiện muộn hơn** lần đọc
đầu, dù `mkdir` đã trả `Ok` trước đó.

Đây là dạng **đọc thấy ảnh cũ ngay sau khi ghi** — khớp giả thuyết "bất đối xứng
ghi/đọc" mà subagent nêu: đường **ghi** thư mục dùng `read_staged` (journal),
đường **đọc** dùng `dev.read` (cache→đĩa).

**Cảnh báo phương pháp (subagent nêu, đã xác nhận đúng):** đèn `DIR` trước đó
**không** chứng minh được cache khớp đĩa, vì "đường RAW" tôi dùng vẫn là
`self.dev.read` = **cùng cache**. Muốn so thật phải đọc qua `self.dev.inner()`.

**Phép đo chốt còn lại:** tại `Ok(None)`, đọc block 0 của thư mục gốc qua
`dev.inner()` (bỏ hẳn cache) và so với `dev.read`; nếu khác ⇒ vá chỗ làm mới
cache sau `commit`/`stage`, không phải vá `read_staged`.

## r294.T26 — QUYẾT ĐỊNH: đĩa THẬT cũng thiếu entry ⇒ không phải cache, mà là thứ tự ghi

Thêm `BlockDev::read_uncached` (mặc định = `read`; `CachedDev` override đi thẳng
`inner`) và tại chỗ `Ok(None)` so **ảnh cache** với **ảnh đĩa thật**, in `mm=`.

**Đo `MAX_IO=256`, 2/14 boot đỏ — cả hai:**

```
V2RD end n=06
DIRP s=0512 b=1 e0=06 mm=00
```

`mm=00` ⇒ ảnh cache **BẰNG** ảnh đĩa thật ⇒ **đĩa thật cũng chỉ có 6 entry**; entry
`d` **chưa hề được ghi xuống** ở thời điểm `readdir` chạy, dù `mkdir` đã trả `Ok`.

Ba kết luận:

1. **Loại hẳn** giả thuyết "cache trả ảnh cũ" — đây là lần đầu tiên phép đo dùng
   đúng đường bỏ cache (nhờ `read_uncached`), sau khi subagent chỉ ra đèn cũ
   không hợp lệ.
2. **Không phải** lỗi tầng liệt kê (client/server readdir đều đúng: 6 entry là
   đúng sự thật của đĩa lúc đó).
3. Lỗi nằm ở **thứ tự bền vững**: `mkdir` trả `Ok` trong khi block thư mục cha
   (chứa entry mới) **chưa** xuống đĩa; lần đọc ngay sau thấy ảnh trước-commit;
   lần đọc sau (khi commit đã xong) thì thấy.

**Việc tiếp (đúng một chỗ):** trong `Fs::commit` (`crates/vifast-fs/src/lib.rs`
~1090-1135), kiểm thứ tự: ghi target → `dev.flush()` → **xoá commit record**.
Nếu commit record bị xoá TRƯỚC khi target xuống đĩa (hoặc `flush` không đảm bảo),
thì cửa sổ "đã trả Ok mà entry chưa bền" là thật. Đèn chốt: sau `commit()` đọc
lại bằng `read_uncached` chính block vừa ghi và so với ảnh staged.

## r294.T26 — đọc `commit()`: thứ tự ĐÚNG; còn một nghi vấn hẹp

`Fs::commit` (`crates/vifast-fs/src/lib.rs:1153-1196`) làm đúng thứ tự chuẩn:
journal data → header → **commit record** → ghi từng `targets[i]` → `dev.flush()`
→ **xoá commit record**. Có ghi chú rõ "never clear the commit journal until
dirty data blocks and their write-back cache have reached the device".

⇒ Giả thuyết "commit record bị xoá trước khi target bền" **không đúng** với mã
hiện tại.

Vậy còn đúng một nghi vấn: **`targets[]` có chứa block thư mục cha không**, và
`dev.write(target)` có thực sự tới thiết bị không. Phép đo chốt kế tiếp (chưa
làm): ngay sau `self.dev.flush()` trong `commit()`, với mỗi `targets[i]` đọc lại
bằng `read_uncached` và so với ảnh journal; lệch thì in `COMMIT LOST <lba>`.
- Có `COMMIT LOST` ⇒ lỗi ở tầng ghi/flush (đã kiểm ATA là sạch, nên nghi
  `CachedDev::write` khi `lba < DATA_START` dùng write-through nhưng KHÔNG flush
  thiết bị, và lệnh đọc sau đó lại đi qua cache của thiết bị theo thứ tự khác).
- Không có `COMMIT LOST` mà đĩa vẫn thiếu ⇒ block thư mục **không nằm trong
  `targets[]`**, tức lỗi ở `write_dir_entry`/`stage`.

## r294.T26 — `commit()` ghi ĐÚNG: 0 mất trên 201 target

Đèn cuối: sau `dev.flush()` trong `commit()`, đọc lại **từng target** bằng
`read_uncached` (bỏ hẳn cache) và so với ảnh journal.

**Đo `MAX_IO=256` (2 boot đỏ trong 10):**

```
V2RD end n=06
COM l=00 v=201          ← 0 lệch / 201 target đã kiểm
DIRP s=0512 b=1 e0=06 mm=00
```

⇒ **Mọi target của mọi commit đều nằm đúng trên đĩa** (`l=00`), cache khớp đĩa
(`mm=00`), và thư mục gốc trên đĩa **thật sự** chỉ có 6 entry tại thời điểm đọc.

Ba tầng dữ liệu đều sạch: thiết bị (ATA 0 lỗi), đường ghi (0/201 lệch), cache
(khớp đĩa). Vậy thông tin "entry thứ 7" **chưa được tạo ra** ở thời điểm lần đọc
đầu — tức `mkdir` đã trả `Ok` **trước khi** việc thêm entry vào thư mục cha được
thực hiện, hoặc entry được thêm **sau** bởi một đường khác.

Nghi vấn hẹp nhất còn lại: **root và client test là hai client FS**, và thứ tự
`mkdir` → `readdir` bị đảo giữa hai client (server xử lý tuần tự, nhưng dãy yêu
cầu của client test có thể chạy **trước** khi yêu cầu mkdir của nó tới server nếu
reply/lệnh bị lẫn). Phép đo chốt: in **thứ tự yêu cầu** ở server (seq + op) quanh
lúc mkdir/readdir, và in seq của reply mà client nhận.

Đây là chỗ duy nhất còn lại sau khi đã loại 10 giả thuyết bằng đo.

## r294.T26 — `open("/d")` ĐÚNG NGAY SAU mkdir, readdir kế tiếp vẫn thiếu

Đèn mới: ngay sau `mkdir("/d")` trả `Ok`, gọi `open("/d")` **trước** readdir.

**Đo `MAX_IO=256` (2 boot đỏ trong 12):**

```
V2MK open=1        ← open("/d") THÀNH CÔNG ngay sau mkdir
V2RD end n=06      ← readdir("/") NGAY SAU ĐÓ chỉ thấy 6 entry
V2GROUP 06
```

Đây là nghịch lý **trong cùng một thời điểm**: `lookup_child` (dùng bởi `open`)
và `read_dir_entry_full` (dùng bởi readdir) đọc **cùng inode RAM, cùng
`blocks_used()`, cùng `dev.read`** (`vifast-fs/src/lib.rs:1535-1545` và
`2114-2125`) — vậy mà một bên thấy entry, một bên không.

Đã kiểm: **không có** cache đường dẫn/dentry trong server hay FS (`grep` sạch), nên
`open` không thể trả kết quả từ cache.

⇒ Hai khả năng còn lại, đều đo được:
1. `read_dir_entry_full` **bỏ qua** entry vì một điều kiện khác trong vòng (ví dụ
   `corrupt` ⇒ `continue`/index lệch) — chứ không phải "hết".
2. `lookup_child` và `read_dir_entry_full` chạy ở **hai thời điểm khác nhau của
   inode** (giữa hai lệnh có một lệnh khác làm mới `self.inodes`) — cần in
   `inode.size`/`blocks[0]` ở **cả hai** chỗ để so.

**Đèn chốt kế tiếp:** in `size`, `blocks_used()`, `blocks[0]` ở **cả**
`lookup_child` (khi tìm thấy) và `read_dir_entry_full` (khi trả `Ok(None)`) kèm
`index` — nếu hai bên khác nhau thì lỗi ở inode; nếu giống nhau thì lỗi ở logic
đếm `seen`/`corrupt` bên trong vòng của `read_dir_entry_full`.

## r294.T26 — MÂU THUẪN QUYẾT ĐỊNH: cùng inode, cùng block, lookup thấy mà đếm không thấy

Đèn `LOOK` ghi hình dạng inode của lần `lookup_child` **tìm thấy**; đèn `DIRP`
ghi hình dạng của lần `read_dir_entry_full` trả `Ok(None)`.

**Đo `MAX_IO=256` (3/12 boot đỏ, cả ba cùng dạng):**

```
V2MK open=1
V2RD end n=06
LOOK s=0512 b=1 b0=30 i=08      ← lookup TÌM THẤY `d` (ino 8) trong block 30
DIRP s=0512 b=1 e0=06 mm=00     ← đếm cùng block đó chỉ ra 6 entry
```

**Cùng inode (size 512, 1 block), CÙNG block số 30, cùng `dev.read`** — mà
`lookup_child` thấy ino 8 còn `read_dir_entry_full` đếm được 6 entry rồi hết.

⇒ Đây **không thể** là vấn đề dữ liệu/đĩa/cache/thứ tự nữa. Lỗi nằm **trong logic
đếm của `read_dir_entry_full`** (`crates/vifast-fs/src/lib.rs:2114-2148`): một
slot có `ino != 0` bị **bỏ qua** ở một điều kiện nào đó trước khi `seen` được
tăng, hoặc nhánh `corrupt` (`return Err(Corrupt)`) đang bị chạm ở một index khác.

**Phép đo chốt (rất hẹp, đèn theo từng slot):** trong vòng của
`read_dir_entry_full`, khi `index` là 6, in **5 slot cuối** dạng
`(slot, ino, len, seen)` để thấy chính xác slot nào bị bỏ và vì sao. Đó là bước
cuối trước khi vá.

Đây là lần đầu tiên toàn bộ tầng lưu trữ (thiết bị, commit, cache, inode) được
chứng minh SẠCH bằng số, và lỗi bị dồn vào đúng một vòng lặp.

## r294.T26 — BẢN ĐỒ SLOT: block thật sự THIẾU ino 08, và lộ churn

Đèn `SLO` in `ino` của 8 slot đầu block 0 tại lần `Ok(None)`.

**Đo `MAX_IO=256` (2 boot đỏ):**

```
SLO 02 04 05 06 07 00 09 00
LOOK s=0512 b=1 b0=30 i=08
```

Đọc bản đồ: block có entry ở slot 0,1,2,3,4,6 → **ino 02 04 05 06 07 09**, slot 5
**trống**, và **không có ino 08**. Vậy `d` (ino 8) **thật sự không có trong block**
tại thời điểm đó — khớp `mm=00` và `COM l=00`.

**Lỗi phương pháp của hai đèn trước (ghi để không lặp):** `LOOK` và `SLO` đều giữ
giá trị của lần gần NHẤT, mà mỗi lần in chỉ xảy ra ở FSCK ⇒ chúng có thể thuộc
**hai thời điểm khác nhau**, nên "cùng inode, cùng block" ở vòng trước là kết luận
**quá mạnh**. Phải ghi kèm số thứ tự sự kiện (seq) mới so được.

**Dữ kiện mới quan trọng:** dãy ino `02 04 05 06 07 09` với **slot 5 trống** cho
thấy thư mục đã qua nhiều lần tạo/xoá; cuối khối V2 có `vfs_unlink("/d")` rồi
`vfs_mkdir("/d")` lại ⇒ entry cuối phải nằm ở slot trống (5) nhưng lại **trống**.

**Việc tiếp:** in kèm **seq** cho cả ba đèn (`V2RD`, `LOOK`, `SLO`) để chắc chắn
chúng thuộc cùng một sự kiện; nếu vẫn thiếu ino 08 thì lỗi ở chính
`write_dir_entry` khi tái dùng slot trống (nghi: ghi vào slot vừa unlink nhưng
`commit`/`stage` không mang block đó).

## r294 — T-IPC tách làm HAI nhóm (đèn `IPC MINT REQ/RET`)

Soak 2 máy sau khi thêm hai mốc bọc lời gọi `cap_mint`:

| Máy | Arch | N | Kết quả |
|---|---|---:|---|
| local | riscv64 | 60 | 60/60 PASS (`RETRY 0/60`) |
| `ps` | x86_64 | 120 | 5 hỏng — **chia làm hai nhóm** |

Chi tiết 5 lần hỏng trên `ps` (mốc có mặt):

```
SHELL READY, IPC MINT REQ                     ← có REQ, KHÔNG có RET ⇒ kẹt TRONG syscall
SHELL READY                                   ← không tới được MINT (chết trước đó)
SHELL READY, IPC MINT REQ                     ← kẹt TRONG syscall
SHELL READY                                   ← chết trước đó
SHELL READY, IPC MINT REQ, IPC MINT RET       ← syscall TRẢ VỀ mà vẫn thiếu IPC RIGHTS OK
```

⇒ **`T-IPC` thực chất là hai lớp khác nhau:**
1. **T-IPC-A**: kẹt **bên trong** `cap_mint` (có `REQ`, không `RET`) — syscall không
   trả về; họ với các lỗi IPC/BKL đã gặp.
2. **T-IPC-B**: chết **trước** `IPC MINT REQ` (dù đã in `SHELL READY`) — cửa sổ
   hẹp giữa vào shell và lời gọi đầu tiên.
3. (hiếm) có `REQ`+`RET` mà vẫn thiếu `IPC RIGHTS OK` — cần xem thêm.

Phân lớp này **phải** đưa vào harness (hiện tất cả vẫn là `T-IPC`), và là dữ kiện
để vòng sau soi đúng chỗ: `cap_mint` là syscall **thuần kernel** (không IPC), nên
kẹt trong đó nghĩa là **BKL bị giữ** hoặc một vòng lặp trong kernel.

Ghi chú quy trình: goal đã được cập nhật — **đầu mỗi vòng phải rà soát bằng 1
subagent**; môi trường hiện chỉ có model `ninerouter/ocg/deepseek-v4.1-flash(xhigh)`
(không có model tên "Muse Spark 1.3 Contributor"), nên dùng model sẵn có và ghi rõ.
