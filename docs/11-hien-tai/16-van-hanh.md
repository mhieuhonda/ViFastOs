# 11.16. Vận hành ViFastOs (runbook)

Cập nhật: vòng 37. Dành cho người trực tiếp build, chạy gate và gỡ lỗi
hàng ngày. Mọi lệnh chạy từ thư mục gốc repo.

## 1. Build và cổng kiểm tra

```bash
export PATH="$HOME/.cargo/bin:$PATH"
./scripts/build.sh                    # build 3 arch + kernel/host tests
python3 scripts/test-qemu.py --repeat 3   # QEMU 3/3×3 (riscv64, aarch64, x86_64)
python3 scripts/test-persist.py       # FS bền 7×3 (replay + corrupt)
python3 scripts/test-harness-spec.py  # 514 ca harness
python3 scripts/gen-docs.py && python3 scripts/check-docs.py
cargo fmt --all -- --check            # 0 warning
```

`./scripts/check-all.sh` chạy toàn bộ cổng trên trong một lệnh.

### 1b. Chạy cổng song song 2 máy

> Quy trình đầy đủ (chuẩn bị, luật chống flake, số đo thời gian) nằm trong
> skill [`skills/vifast-parallel-gate/SKILL.md`](../../skills/vifast-parallel-gate/SKILL.md).
 (giảm ~½ thời gian)

Máy phụ (`ps`, Linux x86_64) đã dựng sẵn: rustup + dải target bare-metal,
`qemu-system-{riscv64,aarch64,x86_64}`, OVMF (`/usr/share/OVMF/OVMF_CODE.fd`
trỏ `OVMF_CODE_4M.fd`), repo ở `~/ViFastOs`.

```bash
./scripts/par-gate.sh --repeat 3        # local: riscv64+aarch64, remote: x86_64
VIFAST_REMOTE=user@host ./scripts/par-gate.sh --repeat 3
./scripts/par-gate.sh --local-only      # fallback một máy
```

Script `rsync` mã nguồn (trừ `target/`), build lại ở máy xa rồi chạy QEMU
x86_64; mỗi kiến trúc có serial/disk/socket riêng nên không tranh chấp. Lưu
ý: `gen-docs.py` cần **một lần** `python3 scripts/test-qemu.py --repeat 3`
đầy đủ trên máy này để ghi fingerprint (`target/qemu-status-standard.json`).


## 1c. Trước khi chạy cổng: không để QEMU mồ côi

Kinh nghiệm vòng 141: script thủ công tự mở QEMU rồi thoát mà **không kill**
tiến trình ⇒ QEMU cũ chạy nền, tranh CPU, làm cổng chuẩn **timeout `PREEMPT
OK`** trên riscv (giống hệt triệu chứng "client quá lớn" ở vòng 116 — lần đó
tôi đã kết luận sai là do kích thước client).

Trước khi chạy `test-qemu.py`/`test-persist.py`:

```bash
./scripts/diag.sh | tail -3     # phải thấy "qemu-system: 0"
ps -Ao pid,comm | grep qemu-system
```

Nếu còn tiến trình: `pkill -f qemu-system` rồi chạy lại cổng.

## 2. Soak (ổn định dài)

```bash
python3 scripts/soak.py --iterations 20            # 3 arch, giữ serial lần hỏng
python3 scripts/soak.py --iterations 50 --archs riscv64,aarch64
```

Kết quả mẫu (vòng 36): 5/5×3 PASS trong 88.8s. Serial lần hỏng nằm dưới
`artifacts/soak/`. Soak không thay thế gate chuẩn.

## 2b. Lệnh shell có sẵn

`help history !! free ps uname uptime time echo ls [path] cat <path> mkdir <path> rm <path>`
`mv <old> <new> ln <old> <new> stat <path> sync <path>`
`truncate <path> <size> df fsck run [path]`
`runfs <path> [args...] exec <path> [args...]`

- `uname` → `ViFastOs <arch> bootinfo-v3`.
- `uptime` → `<giây> s (<n> ticks @ <hz> Hz)`; hz do platform công bố qua
  `Platform::tick_hz()` (mặc định 100 Hz — cả ba port đang đúng nhịp này).
- Sau `SHELL READY`, client gọi syscall `ModuleRelease` (55) để kernel thu hồi
  các trang staging module (riscv thu 21, aarch64 22, x86_64 23 trang) — xem
  mục "Ngân sách cứng đã đo".
- `history` → `last: <lệnh trước>`; `!!` → chạy lại lệnh trước (không ghi `!!`
  vào ô nhớ). Ô nhớ 32 byte, dùng con trỏ thô tới `static mut` để không phát
  cảnh báo.
- `free` → `<trống>/<tổng> KiB` của **arena lớn nhất** (Untyped gốc), qua
  syscall `MemInfo` (53). Đo thật: `24/344 KiB free` ≈ 6 trang còn trống.
- `ps` → mỗi dòng `<slot> <prio> <state>` cho từng TCB đang sống (state theo
  `ThreadState`: 1=Ready, 2=Running, 4=BlockedRecv, 12=Exited…), qua syscall
  `TcbInfo` (52).
- `stat <path>` → `<size> bytes file|dir|link`.
- `df` → `<đã dùng>/<tổng>` block vùng dữ liệu (512 B/block), qua
  `vfs::STATFS`.
- `fsck` chạy kiểm tra toàn vẹn; `time` in tick monotonic.

## 3. Shell lệnh `run` (exec demo)

Trong shell QEMU (interactive):

```text
shell> run
run sent
...
RUN OK
```

Luồng: shell gửi `LABEL_RUN` qua cap lệnh (slot 12, badge 7) tới
fault_endpoint của root → root `spawn_elf_copy` ELF minimal (exit ngay)
vào VSpace mới, hoặc `tcb_restart` TCB cũ nếu đã có. `RUN SKIP` =
hết tài nguyên (fail-êm, không panic). Chạy `run` liên tiếp nhiều lần
an toàn: VSpace setup một lần, mỗi lần chỉ restart TCB.

### 3b. `runfs <path> [args...]` — chạy chương trình từ filesystem

```text
shell> runfs /prog.elf hello
runfs sent
RUN FS OK
hello
```

- Root **tạo `/prog.elf` on-demand** (lần đầu) rồi nạp **từng page từ file**
  vào frame đích (`spawn_elf_from_fs`), không staging.
- `args` được gửi dạng `path\0args\0`; root ghi cặp `(ptr,len)` + chuỗi vào
  trang code, chương trình đọc cặp đó và `DebugWrite` → in đúng tham số
  (cùng binary, nội dung do tham số quyết định).
- Cổng chuẩn tự chạy đường này sau thu hồi pager và in `HIARG` (marker chỉ
  đến từ argv, không nhúng trong ảnh trên đĩa).

#### Ngân sách bộ nhớ (số đo thực, riscv64)

- Pool dữ liệu người dùng: `USER_DATA` = **352 256 B = 86 trang**;
  `DATA_MAX_OBJECTS = 86`.
- Cuối boot (sau toàn bộ demo, trước mọi exec): kernel in
  `DATA UNTYPED pages=80/86 objects=77/86` ⇒ còn **~6 trang + 9 object**.
- Một lần exec (vspace + code frame + stack frame + TCB + page table) tiêu thụ
  ~5 trang/object ⇒ lần exec thứ hai trong cùng phiên **hết chỗ** (đúng giới
  hạn ở mục trên). Vì vậy mọi thêm object vào đường boot đều phải cân nhắc.
- **Ngân sách trang client ELF (theo arch, đo từ bản xanh): riscv64 ≤ 6,
  aarch64 ≤ 7, x86_64 ≤ 8**. Mỗi trang client bị staging cấp 1 Frame lúc boot;
  vượt ngân sách sẽ làm demo `PREEMPT OK` timeout (đã xảy ra ở vòng 116).
  `scripts/build.sh` **chặn ngay** với thông báo `CLIENT ELF <arch> VƯỢT NGÂN
  SÁCH TRANG`.
- `scripts/diag.sh` in nhanh: HEAD, cây sạch/bẩn, kích thước + số trang từng
  module, headroom kernel riscv, trạng thái cổng gần nhất, số QEMU đang chạy.
- Ngưỡng build: headroom kernel riscv phải ≥ **192 KiB** cho cả bản thường và
  bản `test-destroy-smp`; hiện 203 120 B (thường) và 199 024 B (destroy-smp).

#### Lỗi đang mở: chương trình exec không đọc được trang stack (vòng 112–114)

Bằng chứng thu thập (riscv64, kernel xanh):

1. `frame_write(stack_frame, 0xFF8, "HI FS")` trả `Ok` (không có `TOP WRITE BAD`).
2. Hằng số layout in ra từ root đúng: `EXEC_STACK_BASE = 68719620096`
   (`0x10_0002_3000`), `EXEC_STACK_TOP = 68719624192` (`0x10_0002_4000`).
3. Chương trình đọc `entry + 0x40` (trang code) → **in đúng** `HI FS`.
4. Chương trình đọc `EXEC_STACK_TOP - 8` (cùng expression `auipc`+`addi`, đặt
   `auipc` ở lệnh đầu để không lệch PC) → **không in gì**, không marker lỗi.

Suy đoán: trang stack được map "thành công" theo API nhưng **không truy cập
được từ user**; task chết im lặng vì TCB exec không có fault endpoint (muốn có
phải cấp cspace cho exec). Bước kế tiếp: thêm `klog` phía kernel trong
`map_frame` cho VA này (leaf PA + window base/end), hoặc cấp cspace tối thiểu
cho exec để nhận FAULT record.

#### Giới hạn đã biết (vòng 108)

- `runfs`/`exec` nạp được **một lần mỗi phiên**; lần thứ hai trong cùng phiên
  hiện **rơi về demo copy** (`RUN OK`) vì Untyped đã sát trần: `destroy_exec`
  không trả trang/page-table về pool, nên `spawn_elf_from_fs` dừng ở
  `map_frame_fallible` (thiếu bộ nhớ để retype frame/page-table).
- Theo dõi bằng: `DATA UNTYPED pages=…/86 objects=…/86` cuối serial. Muốn lặp
  nhiều lần phải làm một trong hai: (a) tái dùng frame + page table của exec
  (unmap → ghi lại → map, không cấp object), hoặc (b) trả trang về pool khi
  `destroy_exec` (cần sửa kernel).

## 4. Đọc serial và chỉ số sức khỏe

Mỗi lần boot in một dòng cho mỗi hạng mục. Các dòng cần có:

- `DATA LEAKED 0` — bytes leak tích lũy của data Untyped (kỳ vọng 0;
  >0 nghĩa là free list đầy khi reclaim, cần dọn).
- `DATA UNTYPED pages=A/B objects=C/D` — mức dùng arena (B=86).
- `BKL STATS (h0_acq=… spin=…)` — contention khóa kernel trên SMP.
- `SMP SCHED OK`, `SMP WORKERS OK`, `SMP POST IDLE OK` — sức khỏe SMP.
- `WAIT REAP OK`, `EXEC OK`/`EXEC SKIP`, `RUN OK`/`RUN SKIP`.

## 5. Xử lý flake thường gặp

| Triệu chứng | Nguyên nhân hay gặp | Cách xử lý |
|---|---|---|
| QEMU timeout (rc=-9) dưới tải cao | load máy >4 làm guest chậm quá 20s | `pkill -f qemu-system`, đợi load <2, rerun |
| Mất byte UART (`run sent` không thấy) | FIFO PL011 16 byte, gửi dồn | rerun; interactive gửi dồn + `\r` |
| `ROOT ERR 5` sau fault3 | data Untyped cạn (arena sát trần) | tăng DataPool + `DATA_MAX_OBJECTS` (đồng bộ 3 arch) |
| `BOOT UNTYPED TOO SMALL` | root ELF to ra, boot arena cạn | tăng BootPool + `BOOT_MAX_OBJECTS` (đồng bộ 3 arch, giữ `MAX_OBJECTS` ≤ budget) |
| PAGER3 fault 2 lần | race evict/demand khi đổi interleaving | bình thường (harness cho phép 1–2) |
| `torn replay boot` persist | flake replay x86 | rerun; FAIL liên tiếp mới điều tra |

### 5w. Tiến trình: huỷ TCB đã được khoá

`KILL OK` — root retype một TCB vào slot vừa trống rồi `tcb_destroy` (phải
thành công), và `tcb_destroy(200)` trên slot không tồn tại phải bị từ chối.
Đây là đường "tiến trình" cơ bản: tạo — huỷ — kiểm tra.

### 5x. Bảo mật cơ bản đã khoá bằng self-test

- `SEC PROBE OK` — kernel **từ chối đọc hộ** con trỏ nằm ngoài cửa sổ copy
  (`USER_VA_START - 1` và `0`); nếu đọc được thì `DebugWrite` thành kênh rò rỉ
  bộ nhớ kernel.
- `SYS REJECT OK` — handle không hợp lệ (`slot 200`) và con trỏ vượt
  `USER_VA_END` đều trả lỗi, kernel vẫn chạy tiếp.
- `WX OK` — ghi vào trang đã map executable bị từ chối.

### 5y. Kiểm tra đĩa ngay khi mount

Server chạy `fsck()` ngay sau khi nạp FS và in **`FS MOUNT CLEAN`**.
Nếu phát hiện bất nhất:
- thư mục gốc **trống** ⇒ format lại và in **`FS MOUNT REPAIRED`**;
- **có dữ liệu người dùng** ⇒ **không đụng vào đĩa**, in **`FS MOUNT DIRTY`**
  (an toàn dữ liệu trước, người dùng tự quyết định). Đây là marker của cổng: nếu một thay
đổi làm cấu trúc đĩa hỏng, sẽ thấy ngay ở dòng này thay vì đoán qua lỗi FS về
sau.

### 5z. Ngân sách cứng đã đo (đừng đoán)

Số đo tại vòng 157 trên cây xanh:

| Hạng mục | Giá trị | Ghi chú |
|---|---|---|
| `vifast-client.stripped` (riscv) | **22 552 B = 6 trang** | ngân sách 6; +~100 B code là có thể vượt trang |
| `vifast-root.stripped` (riscv) | **37 376 B** | |
| `vifast-root.smp.stripped` | **37 376 B** (bình thường) | **+6 dòng code ⇒ 41 472 B (+1 trang)** |
| headroom kernel riscv (cả 2 bản) | **2 296 176 B** (từ vòng 158) | ngưỡng 192 KiB (196 608) |
| ghi chú | cửa sổ kernel riscv đã nới 2 MiB → 6 MiB ở vòng 158 | thêm superpage 0x8040_0000 |

Hệ quả thực tế: một thay đổi "nhỏ" ở root hoặc client có thể **vượt biên trang**
và làm kernel nhúng ELF phình thêm 4 KiB, đủ phá ngưỡng headroom. Vì vậy:

1. Đo ngay sau mỗi lần sửa: `./scripts/diag.sh` (in số trang + headroom).
2. Thêm tính năng shell phải tính **cả code lẫn bảng tĩnh** (ví dụ bộ đệm lịch sử
   4×32 B đã đẩy client 22 552 → 25 088 B = **7 trang**, vượt ngân sách và bị
   guard chặn — đã bỏ tính năng đó ở vòng 157).
3. Nếu buộc phải tăng, nới **có kỷ luật**: đo lại 3 arch, cập nhật ngưỡng trong
   `scripts/build.sh` và ghi vào log — không nới ngầm.

### 5a. TUYỆT ĐỐI không build tay đè kernel của cổng

`scripts/build.sh` build kernel **có feature `test-exit`** (mặc định của crate)
và ghi vào `target/<triple>/release/vifast-kernel-*`. Nếu bạn chạy tay

```bash
cargo build -p vifast-kernel-riscv64 --release --no-default-features
```

thì **file đó bị ghi đè** bằng bản KHÔNG có `test-exit`; sau đó mọi lần chạy
cổng sẽ đỏ với `thiếu 1 marker: PREEMPT OK` — dù hệ thống hoàn toàn khoẻ. Lỗi
này đã làm tôi đi sai hướng ở vòng 149/151/153.

Từ vòng 153 `test-qemu.py` **tự kiểm tra**: nếu binary thiếu chuỗi `PREEMPT`,
nó dừng ngay với thông báo *"chạy ./scripts/build.sh trước khi chạy cổng"*.
Khi cần thử kernel đặc biệt, dùng `--target-dir` khác hoặc build xong chạy lại
`./scripts/build.sh` trước khi chạy cổng.

### 5b. Ba lớp lỗi đã gặp nhiều lần (phân biệt nhanh)

| Triệu chứng | Cách phân biệt | Việc cần làm |
|---|---|---|
| `thiếu 1 marker: PREEMPT OK` (riscv) | chạy lại **một** lần; nếu PASS ⇒ flake | `./scripts/diag.sh` phải báo `qemu-system: 0`; nếu còn ⇒ `pkill -f qemu-system` |
| `guest không boot (chỉ N dòng serial)` | N rất nhỏ (vài dòng) | flake UEFI/QEMU: `test-persist` đã tự thử lại 1 lần; `test-qemu` chạy lại |
| `CLIENT ELF <arch> VƯỢT NGÂN SÁCH TRANG` | build fail ngay | thu gọn `vifast-client` hoặc nới Untyped; **đừng** nâng ngân sách mà chưa đo QEMU 3/3 |

Ghi chú lịch sử: lỗi "exec không đọc được trang stack" (vòng 112–145) hoá ra là
**thiếu cửa sổ VA trong `read_user`/`write_user`** của platform, không phải lỗi
bảng trang — xem `docs/bugs/exec-stack-page-unreadable.md` (đã đóng). Hợp đồng
cửa sổ giờ nằm ở `vifast_abi::user_layout::COPY_WINDOWS` và có test host canh.

## 6. Mở rộng arena (quy trình)

Arena nằm ở `platform-*/src/platform.rs` (`DataPool`, `BootPool`,
`METADATA`/`USER_DATA`), quota ở `vifast-runtime/src/lib.rs`
(`DATA_MAX_OBJECTS`, `BOOT_MAX_OBJECTS`, `META_MAX_OBJECTS`) và tổng
bị chặn bởi `MAX_OBJECTS` trong `vifast-kernel/src/object.rs` cùng
`KERNEL_STATIC_BUDGET` (aarch64 còn kiểm tra gap BKL trong `build.sh`).

1. Tăng pool cả 3 arch đồng bộ (riscv, aarch64, x86_64).
2. Tăng quota tương ứng, giữ `RUNTIME+META+DATA+BOOT ≤ MAX_OBJECTS`.
3. `./scripts/build.sh` phải qua `AArch64 static check` (gap >0).
4. Chạy gate đầy đủ + soak ngắn trước khi commit.

## 7. Backup/restore FS image

Disk test do `test-qemu.py` tự tạo (`make_disk`, 64 MiB, raw). Giữ bản
tốt khi cần đối chiếu:

```bash
cp /tmp/vifast-riscv64.disk ~/vifast-backup-riscv64.disk
```

Corrupt test có chủ đích nằm trong `test-persist.py` phase 6 (superblock),
không chạy tay lên disk thật.
