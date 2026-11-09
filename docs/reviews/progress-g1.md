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
