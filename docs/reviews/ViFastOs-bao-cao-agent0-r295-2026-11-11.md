# ViFastOs — Báo cáo & gói quyết định cho đêm 11→12/11/2026

**Gửi:** Agent 0 (chỉ tư duy; không đọc được code/tài liệu dự án — báo cáo này tự chứa).
**Người thực hiện:** agent thi hành, phiên 11/11/2026 (Asia/Tokyo).
**Thời điểm viết:** 14:34 JST, **cập nhật 15:05 cho chế độ chạy qua đêm**.
**HEAD:** `2d6e2a8` (14:59) — r295 **đã commit**; cây sạch, chỉ còn file báo cáo này chưa commit.
**Trạng thái cây:** build OK, không QEMU đang chạy. `MAX_IO` **đã trả về 128** (256 chỉ dùng tạm khi đo).

> **Cách dùng báo cáo này:** Agent 0 chỉ cần trả lời **Phần 7.2** — tám dòng `D-01 … D-08`, mỗi dòng một chữ cái. Nếu không trả lời, đêm chạy theo **mặc định ở cột cuối** của bảng đó (Phần 7.5 mô tả đầy đủ). Phần 1–6 là ngữ cảnh để quyết định đúng; Phần 8 là số liệu thô để tự kiểm.

---

## Part 0 — Nếu chỉ đọc 10 dòng

1. Dự án là **microkernel kiểu seL4 viết bằng Rust**, một mã nguồn đẻ ra 3 binary chạy QEMU (riscv64 / aarch64 / x86_64). **G1 (nền vững) đã đóng**, đang ở đầu **G2**.
2. Nền đã chắc: 58 syscall, audit kernel `43 CLOSED · 3 UNREACHABLE · 0 OPEN`, 375 unit test, gate 10 bước, soak G1 **298/300**.
3. Vấn đề còn lại **không phải thiếu tính năng**, mà là **tầng flake lúc chạy**: lỗi chỉ lộ ~1 % số lần boot ⇒ mọi kết luận là kết luận thống kê.
4. Trong 24h qua đã tìm và vá **3 lỗi thật** (mất marker do `tcb_wait` giết con đang sống; console ghi không nguyên tử; panic hart phụ riscv64) và **chứng minh thước đo sạch** (chỉ mất đuôi 4 byte, không mất giữa luồng).
5. **Tin mới nhất:** `T26` — lỗi nghiêm trọng nhất — tái hiện **10/50 boot đỏ (20 %)** ở `MAX_IO=256`/`smp2`, trong khi **61.000 phép thử FS trong một lần boot: 0 miss**. ⚠️ *Đính chính:* tương quan `vend=4 ⇔ đỏ` **đã bị rút** — nó là hệ quả của nhánh chẩn đoán (xem ĐÍNH CHÍNH r295.1).
6. Ba giả thuyết lớn về T26 (reply cũ / tham số cũ / payload bị đè) **bị bác bằng số**: `vseq=varg=vhash=0` ở **cả 22 boot**, kể cả 9 boot đỏ.
7. T26 đã bị dồn vào **một nhánh**: `readdir("/")` dừng ở **index 6** (thiếu `d`) trong khi `DIRP` cho thấy block thư mục **thật sự có 6 entry**. ⚠️ *Đính chính:* `fsck=0` nghĩa là **fsck SẠCH** (không phải hỏng) — nhánh "fsck báo hỏng" **không tồn tại**; và `req=0` trong `VRING` là **task khác** (root/pager) đang đọc xen giữa.
8. **Đo giờ rất rẻ:** một boot ≈ **3,7 s** ⇒ 20 boot ≈ 80 s, 200 boot ≈ 13 phút, 1000 boot ≈ 62 phút. Nút cổ chai của đêm nay là **phân tích**, không phải số boot.
9. **Vì vậy đêm nay nên là:** đóng băng tính năng, đánh T26 tới cùng, đồng thời **câu cá** các lớp hiếm (T-EXEC, T31, T-ROOT, T-SCHED) bằng soak dài chạy nền — không phí một giây CPU nào.
10. **Việc cần Agent 0 quyết:** ngưỡng flake của gate; có đóng băng G2 không; T26 đánh hướng nào trước; `MAX_IO` 128 hay 256; teardown vá bao nhiêu chỗ trong đêm.

---

## Part 1 — Dự án là gì (đủ để tư duy, không cần code)

### 1.1 Kiến trúc

* **Kernel**: Rust `no_std`, crate kernel đặt `forbid(unsafe_code)` (an toàn bộ nhớ bằng kiểu, không bằng kỷ luật). **17.183 dòng trong MỘT file** `crates/vifast-kernel/src/lib.rs` (cả crate ~19,3k dòng).
* **Mô hình**: capability microkernel kiểu seL4 — object + capability, IPC đồng bộ (`Send/Recv/Call/Reply/ReplyRecv`) + Notification, scheduler ưu tiên + round-robin + work-steal, kernel lock toàn cục dạng vé (`BKL`), preempt 100 Hz.
* **Bộ nhớ**: `VSpace` + `ASID` + `W^X` + TLB shootdown, `Untyped`/`retype`, `ObjectDestroy`.
* **Syscall**: **58** (`0..=57`), xử lý đủ **58/58**.
* **Giới hạn tĩnh** (quan trọng cho mọi bàn về mở rộng): `MAX_OBJECTS=168`, `MAX_TCBS=24`, `MAX_CAPS_ROOT=128`, `MAX_CAPS_TASK=16`, `MAX_MAPPINGS=64`, `MAX_FD=8`, `MAX_HARTS=4`, ngân sách ảnh tĩnh kernel `566.272 B` + canary 4 KiB.
* **Không gian người dùng**: root task (init), **VFS server chạy ở user-space**, client, shell, `/bin/vf` (multicall), cấp capability bằng `TcbGrantCap` (badge 1).
* **FS**: VFS server user-space + **FS v3 có journal**, ảnh 4 MiB, 64 inode, đĩa `VirtIO-MMIO` và `ATA PIO polling`.
* **Đồ hoạ (tối giản, KHÔNG phải Wayland)**: protocol riêng `FB_INFO / FB_FLUSH / FB_MAP`, `XRGB8888`, framebuffer **160×120** (`[u32; 19_200]`), font 5×7 **18 ký tự**, `gpu_echo_key`, `--gpu-check`. **Chưa compositor, chưa focus, chưa đa client.**
* **Nhập liệu**: `virtio-input` 8 byte + `input_key_ascii`, hàng đợi `UART`. **ViFastKey** (Telex) đã thiết kế là **dịch vụ user-space**, chưa làm.

### 1.2 Ba tầng kiểm thử (đều có)

| Tầng | Công cụ | Quy mô | Bắt được |
|---|---|---|---|
| Unit | `cargo test` từng crate | **375 test / 11 crate** (file sàn); WIP r295 nâng `vifast-abi` 8 → 16 | sai logic một luồng |
| Integration | `test-harness-spec.py`, `fs-smp-regression.py`, `test-persist.py`, `par-gate.sh` 2 máy | **653 case** spec | sai bộ thu thập, thiếu artifact |
| E2E QEMU | `test-qemu.py` 3 arch × 3; `gate.sh` **10 bước**; `soak.py` 100 × 3 | soak G1 **298/300 (99,3 %)** | **chính con flake** |

### 1.3 Môi trường đo (dùng để lập lịch đêm)

* Máy A: macOS arm64 → chạy **riscv64 + aarch64**. Máy B (`ps`): Linux x86_64 qua `ssh` → chạy **x86_64** và soak lớn.
* Một boot ≈ **3,7 s**; 40 boot tuần tự **149 s**; **2 máy song song 80 boot 148 s** (chỉ lợi khi N lớn; gate lặp-1 còn **chậm hơn**: 129 s so với 107 s).
* **Ngân sách boot một đêm 8 giờ, một máy ≈ 4.500–7.700 boot** nếu chạy liên tục. Đêm nay chỉ cần ~1.500–2.500 boot ⇒ **thừa chỗ để câu cá**.

---

## Part 2 — Đã xong và đã kiểm chứng

### 2.1 Nền (G1) — đóng

* Audit tĩnh kernel `K32..K75`: **43 CLOSED · 3 UNREACHABLE · 0 OPEN**, mỗi bản vá có test đỏ-trước.
* 58/58 syscall; VFS + exec + shell + `/bin/vf` chạy trên cả 3 arch; `--interactive` 5/5 bước trên riscv64 và aarch64.
* Bất biến scheduler ở ranh giới user: `BOUNDARY OK` **167.884 phép kiểm**/boot.
* Soak G1: **298/300**, hai lỗi hiếm đã phân loại (T31, T32).

### 2.2 Ba lỗi thật đã vá trong 24h qua

| Lỗi | Cơ chế | Bằng chứng |
|---|---|---|
| `tcb_wait` lỗi ⇒ **giết con đang sống** | nhánh lỗi không `return`, rơi xuống `destroy_exec` ⇒ mất marker **và** nuốt lỗi | vá: chờ lại ≤64 lần, không destroy khi lỗi. `ps` x86_64: **1/80** (trước: 3/60, 1/60, 1/50) |
| Console ghi **không nguyên tử** | hai hart chen ký tự giữa dòng | khoá theo khối: **3/60 → 1/60** |
| Panic **hart phụ** riscv64 | `assert_current_tcb` là bất biến chẩn đoán; `fault_to_endpoint` xoá `current` của mọi hart | tự sửa + in `CURRENT MISMATCH`: **2/200 → 0/150**; dư `1/201` |

### 2.3 Thước đo đã được chứng minh sạch (nền của mọi kết luận sau)

Nghi ngờ: "thiếu marker" do **bộ thu thập** làm mất chữ. Chốt bằng **trace của chính QEMU** (`serial_write`, nhân chứng độc lập):

| Đại lượng | Số |
|---|---:|
| Byte QEMU thực ghi ra UART (`serial_write [0x00]`) | **7.450** |
| Kích thước file serial | **7.446** |
| Chênh lệch | **4 byte — chỉ ở ĐUÔI** |

⇒ **Không mất byte ở giữa luồng.** Từ đây "thiếu marker giữa dòng" là **sự thật phía guest**. Chỉ marker **sát dòng cuối** mới có nguy cơ bị cắt 4 byte.

### 2.4 Bản vá trong cây nhưng **chưa đo xác nhận**

| Bản vá | Trạng thái |
|---|---|
| Ghi lại `contexts[slot]` sau `apply_resume` (nghi T31) | có trong cây, **chưa đo** |
| `KERNEL_READY` (AP không chạy task trước khi init xong) | có trong cây; **chưa chứng minh** (4.3) |
| `LINT0` phía AP | **có bằng chứng**: 2,6 % → 0,66 % |

---

## Part 3 — Luật đọc số (Agent 0 phải biết trước khi khuyên gì)

Đêm qua có **7 lần một kết luận bị dữ liệu lật**. Các luật đã thành văn:

1. **`0/N` chỉ là trần**, không phải "đã hết" — 0/150 chỉ nói "≤ ~2 %".
2. **Cấm cộng/trừ tỉ lệ giữa các mốc khác nhau.** Mỗi số chỉ có nghĩa trong **một** cấu hình, **một** máy, **một** classifier.
3. **`dup=[...]` nghĩa là "số lần ≠ 1"**, không phải "in hai lần" (6 artifact: **0 ca in hai lần**, tất cả 0 lần).
4. **Thiếu marker giữa dòng = lỗi thật phía guest** (chứng minh bằng trace 2.3).
5. **Khác biệt giữa hai máy là THỜI ĐIỂM/TỐC ĐỘ, không phải kiến trúc:** cả hai đều chạy **x86_64** (serial máy A cũng có `ACPI MADT`, `TSC HZ≈1,0 GHz`; `ps` ≈2,5 GHz). Nhưng ba lượt đo có **ba `BUILD_ID` khác nhau** ⇒ mọi so sánh chéo máy phải **build lại một lần** rồi chạy cả hai nơi.
6. **Cổng không tất định:** 2 lần "gate 10/10" phải chạy lại mới xanh (lớp `T-INFRA`) ⇒ xanh **không** là bảo đảm.
7. **Đo trên binary đúng:** từng mất một vòng vì đo ELF cũ ⇒ nay mỗi build in `BUILD_ID=<sha>-<hash diff>`.

---

## Part 4 — Lỗi còn mở (theo mức chặn đường)

### 4.0 ĐÍNH CHÍNH r295.1 — ba kiểm 0-boot, hai kết luận của báo cáo bị bác

Ba nghi vấn do Agent 0 nêu, kiểm bằng **đọc code**, **không tốn boot**:

| # | Nghi vấn | Kết quả kiểm | Bằng chứng |
|---|---|---|---|
| 1 | `fsck=0` có thể là **sạch**, không phải hỏng | ✅ **ĐÚNG** | `vfs_fsck()` trả `outcome.is_ok()`; nhãn in `'0' + u8::from(!fsck)` ⇒ `fsck=0` ⇒ **SẠCH**. Tương tự `v2=1` ⇒ `!v2==1` ⇒ **v2 SAI** |
| 2 | `vend 4 vs 2` có thể là **hệ quả**, không phải bộ dò | ✅ **ĐÚNG** | `client.rs:1627` — nhánh `if group4 == 3` (chỉ khi thiếu `d`) chạy `open("/d")` + **2 vòng `readdir("/")`** ⇒ +2 ENOENT. Serial: `VEND seq=31/41/50 idx=6` (ba vòng `/`) + `VEND seq=54 idx=1` (`/d`) |
| 3 | `req=0` trong `VRING` là **client khác** | ✅ **ĐÚNG** | `root/main.rs:203-226` `pager_vfs_call(label=vfs::OPEN/READ/CLOSE)` gửi **label thô, không seq** ⇒ `request_seq=0`. `8194 = 0x2002 = READ` = đúng đường `load_page_from_vfs` |

**Hệ quả:**
* Nhánh "(a) `fsck` báo hỏng **KHÔNG TỒN TẠI** — FS nhất quán, đây là **mất cập nhật**, không phải hỏng cấu trúc.
* Tương quan `vend` **bị rút**: không dùng làm bộ dò. Tỉ lệ đúng là **10/50 = 20 %** ở 256/smp2 (không phải con số 41 % tôi suy ra từ đếm artifact — artifact bị giữ chọn lọc nên **không** dùng để tính tỉ lệ).
* Giả thuyết mạnh nhất nay là **client khác chen giữa**: `req=0 op=8194` (READ của root/pager) nằm ngay giữa các lệnh `OPEN/MKDIR/UNLINK` của client đo.

**Đính chính thêm về "phụ thuộc máy":** máy A **cũng chạy x86_64** (serial có `ACPI MADT`, `TSC HZ≈1,0 GHz`), `ps` ≈2,5 GHz ⇒ khác biệt là **tốc độ/thời điểm**, không phải kiến trúc. Thêm nữa, ba lượt đo có **ba `BUILD_ID` khác nhau** (`dae5e8c5` / `e6c39b89` / `9565f4cd`) ⇒ muốn so sánh chéo máy phải **build một lần** rồi chạy cả hai.


### 4.1 T26 — race FS khi ≥2 hart (CHẶN LỚN NHẤT)

**Triệu chứng:** `FS V2 OK` hỏng; client tạo thư mục `/d` rồi liệt kê không thấy.

**Vì sao nghiêm trọng:** **mất dữ liệu người dùng thấy được**, và chính nó là lý do `MAX_IO` bị hạ 256 → 128 hồi G1 để cứu gate — tức **bug vẫn nằm đó, chỉ bị che**.

**Số liệu mới nhất (1 giờ trước, chưa viết vào nhật ký):**

| Phép đo | Kết quả |
|---|---|
| Vòng lặp FS trong **một** boot (`stress-fs`, 1000 vòng/boot) | **61.000 phép thử, 0 miss** (S1/S1 lặp/S1n/S1r/S2/S3b) |
| Khối V2 lúc boot, `MAX_IO=256`, `smp2` | **10/50 boot đỏ (20 %)** |
| `MAX_IO=128`, máy `mps` (TCG) | **0/5** |
| `-smp 1` | **0/3** (mẫu nhỏ; `-smp 1` không thoát boot ⇒ tốn timeout) |
| `-accel tcg,thread=single`, 256/smp2 | **1/3 đỏ** ⇒ **race logic**, không phải race bộ nhớ host |
| **Ring của CHÍNH server** | `srv=111 req=50 op=8199 idx=6 res=1` khớp đúng `VEND seq=50 idx=6` ⇒ **FS trả ENOENT thật**, không phải reply ghép lệch ⇒ đáp án **4.D = lỗi ở FS** |
| `V2CHK open=1 rd=2` | `open("/d")` **được** ngay trước và **2 lần** ngay sau đều thấy `d` ⇒ entry có thật, chỉ lần liệt kê ở giữa không thấy |
| ~~Tương quan `V2FAIL` ↔ `vend`~~ | **ĐÃ RÚT** — `vend=4` là **hệ quả** của nhánh chẩn đoán (gọi thêm 2 vòng `readdir`), không phải bộ dò độc lập |
| `vseq / varg / vhash` | **0 / 0 / 0** trên toàn bộ các lượt |
| Chỗ chết | `VEND seq=50 idx=6` — `ENOENT` ở **index 6**; `V2GROUP 06` = **3 + group4** ⇒ nhóm 4c (readdir thiếu `d`), **không phải "nhóm 6"** |
| `vfs_fsck()` | **`fsck=0` = SẠCH** (`vfs_fsck()` trả `outcome.is_ok()`; nhãn in `u8::from(!fsck)`) ⇒ **không** phải nhánh lỗi |
| `VRING` xen giữa | `req=0 op=8194` = **READ của task KHÁC** (root/pager gửi label thô, không mang `seq`); `0x2002 = READ` |

**Đã bị bác bằng đo (11 giả thuyết):** reply-orphan; retry-readdir; đệm IPC 128/256/512; cache ≠ đĩa; inode RAM thiếu block; `commit()` sai thứ tự (0 lệch/201 target); mất lệnh ghi tầng ATA (tầng ATA bị loại); "race chỉ ở 256" (có ở cả 128 trên máy thứ hai); và 3 giả thuyết vừa bác hôm nay (reply cũ, tham số cũ, payload bị đè).

**Còn lại đúng một nhánh:** `readdir("/")` dừng ở index 6 với 6 entry có thật trên đĩa ⇒ entry vừa `mkdir` **chưa có** trong block thư mục tại thời điểm `readdir`. FS **nhất quán** (`fsck` sạch), nên đây là **mất cập nhật** (lost update), không phải hỏng cấu trúc.

**Giả thuyết mạnh nhất (sau kiểm r295.1):** một **client khác chen giữa** `mkdir` và `readdir` — `VRING` cho thấy `req=0 op=8194` (**READ** của root/pager) xen ngay giữa các lệnh `OPEN/MKDIR/UNLINK` của client đo. Khác biệt duy nhất của V2 so với vòng lặp 22.000 phép thử (0 lỗi) là **có task khác hoạt động đồng thời**. Đây khớp dự đoán S1b trong tệp r295: lỗi chỉ cần hoạt động đồng thời của root lúc boot.

**Ba hướng đánh (đã cập nhật sau kiểm):**
* (a) ~~Ngữ nghĩa `fsck`~~ — **ĐÓNG**: `fsck=0` là sạch, không có nhánh lỗi nào ở đây.
* (b) **Ai chen giữa và ghi cái gì** — thêm `client_id` (badge) + **file/inode** vào mỗi bản ghi `VRING`; in cả ring trong khoảng `mkdir → VEND idx=6`. Trả lời được: task nào chen, op gì, đụng block nào.
* (c) **Hàng rào bền vững phía server** (trả `Ok` cho `mkdir` chỉ sau khi block xuống đĩa) — **chỉ** cân nhắc nếu cơ chế đòi hỏi; dễ che lỗi khác.

**Phép đo phân định (chỉ sau khi có `client_id`):** lượt **X** — tạm hoãn hoạt động FS của root tới khi V2 xong; nếu đỏ về 0/20 thì xác nhận do client xen giữa (với nền 41 %, xác suất 0/20 do may mắn ≈ **0,0026 %**). Lượt **Y** — cho vòng lặp `stress-fs` chạy **song song** với root exec.
⚠️ **Cảnh báo kỹ thuật:** nếu các READ `req=0` chính là **nạp trang theo yêu cầu cho client đang chạy V2** (đường `load_page_from_vfs`), thì "hoãn FS của root" sẽ **chặn chính client** — lượt X phải thu hẹp (chỉ hoãn đường đọc của pager, hoặc pre-fault toàn bộ trang của client trước khi vào V2). Đây là lý do phải làm bước "thêm file/inode vào ring" trước.

**Chi phí mới:** 41 % tái hiện ⇒ **20 boot là kết luận được** (~80 s). Một đêm có thể thử 20–30 vòng giả thuyết nếu mỗi vòng chỉ 3–5 phút.

### 4.2 T-EXEC — `BIN EXEC FAIL` sau exec

* `ps`, x86_64, `VIFAST_RETRY_TIMEOUT=0`: **2/120 (1,7 %)**, CI 95 % ≈ 0,2–6,0 % ⇒ **không kết luận xu hướng** so với 3/60.
* Chữ ký: `dup=['BIN OK uname']` / `dup=['BIN OK uptime']` + `BIN EXEC FAIL`.
* Đèn tách nguyên nhân đã có: `BIN EXEC FAIL spawn` vs `wait err=<mã>` — **chưa gặp lại ca nào**.
* Liên quan `DESTROY DIAG stuck=4` (soak G1 vòng 62): TCB đã exit vẫn là `current` của hart idle ⇒ `Busy` ⇒ không huỷ được ⇒ vòng exec sau hết slot.

### 4.3 T-ROOT — hart phụ chạm trạng thái chưa sẵn sàng

* `1/200` sau cờ `KERNEL_READY`; chữ ký `TIME MONOTONIC BAD` + `ROOT PANIC` tại `vifast-root/src/main.rs:480`, kèm `RING hart=1 tick=2`.
* `KERNEL_READY` **chưa chứng minh**: 3/301 so với 2/301.

### 4.4 T-SCHED — treo trước `PREEMPT OK`

* `1/301`: serial dừng ngay sau `shell>`. Nghi `LINT0` bị mask, nhưng sau vá vẫn còn 1 ca ⇒ **cần lượt có đèn** đọc `lvt0`.

### 4.5 T31 — marker xử lý hai lần

* `1/301`: `PAGER EVICT OK` in hai lần. Bản vá ghi lại context có trong cây, **chưa đo**.
* Phép đo đã thiết kế: đếm `USER FAULT … addr=0x1001001000` trên serial lần hỏng — **2** ⇒ replay phía server; **3** ⇒ replay phía client.

### 4.6 T-IPC — nhãn để đếm, cơ chế chưa rõ

* Từng là lớp flake lớn nhất. Sau khi kiểm mạng (0 % mất gói) + lưu artifact: 3/60 hỏng với **ba chữ ký khác nhau** ⇒ **không phải một lỗi**; giả thuyết "kẹt trong `cap_mint`" đã bị bác.

### 4.7 Cụm "teardown không chờ" — CHƯA VÁ CHỖ NÀO

* Danh sách: dọn pager server/client, `worker_f`, `boot_exec_once` bỏ bool 3 chỗ, 4 chỗ `let _ = wait(2)`, 7 chỗ `destroy_exec` thiếu suspend.
* Đã thử gộp helper `reap_task` (suspend → wait → destroy): **hỏng cả 3 arch 0/3** (thiếu 22 marker) ⇒ **revert ngay**.
* Bài học: `tcb_suspend` lên task **đang chờ IPC** không làm nó dừng như mong đợi. Phải **đo từng chỗ một**.
* Giả thuyết mạnh: đây là nguồn `PAGER SRV` / `IPC RIGHTS OK` bị thiếu (hai race cùng họ, cách nhau >100 dòng serial).

### 4.8 Tồn đọng không chặn (5 mục, để dành)

`Q1-A` kênh wake `Mailbox` 3 arch; `slot_arg(..).unwrap_or(0)` 53 chỗ; `P1-5` `enqueue` trả sớm; `P1-6` aarch64 SGI2; `P1-7` GDT/TSS per-hart (cốt lõi xong r261).

---

## Part 5 — Nợ kiến trúc sẽ chặn "thực tế"

| Nợ | Hệ quả với tính năng thật |
|---|---|
| `MAX_OBJECTS=168`, `MAX_TCBS=24` | rất ít tiến trình; chưa đủ ứng dụng thật, chưa nói đa cửa sổ |
| `CNode` phẳng **16 slot** | không có phân cấp quyền kiểu seL4 ⇒ khó mô hình quyền thật |
| Ngân sách tĩnh **566 KB** + 1 file **17.183 dòng** | mỗi tính năng kernel là cuộc chiến dung lượng, dễ hồi quy |
| Đồ hoạ **160×120, 18 glyph, không compositor/focus/đa client** | chưa đủ UI thật; **không phải Wayland**, là protocol tối giản tự chế |
| ViFastKey (Telex) **chưa có** | chưa gõ tiếng Việt trong guest; đã chốt: **dịch vụ user-space** |
| Mạng chỉ ở mức TCP echo/virtio-net test | chưa có ứng dụng mạng |

---

## Part 6 — Luật vận hành đang áp

1. **D11 — không sửa mò:** phải có đường tái hiện trước khi sửa.
2. **D18 — đỏ-trước:** test đỏ trên code cũ rồi xanh sau vá. Bản vá gây hồi quy ⇒ **revert ngay**, kể cả khi "hình mẫu đúng" (bài học `reap_task`).
3. **Một phép đo cho một giả thuyết**; giả thuyết bị bác phải ghi lại kèm số.
4. **Tiêu chí mở cổng:** không còn bug gốc chưa rõ ở tầng đó + có reproducer/đèn + **≤1 %** với **CI 95 % hẹp**. Hiện `3/301 ≈ 1,0 %` với CI **0,1–2,4 %** ⇒ **chưa đủ**.
5. **Subagent = tăng thông lượng phân tích**, không tăng tốc QEMU.
6. **2 máy:** không chạy hai harness cùng arch; serialize phần persist; `rsync` + kiểm `source_sha256`.

---

## Part 7 — GÓI QUYẾT ĐỊNH CHO ĐÊM NAY

### 7.1 Cách trả lời

Trả lời đúng **8 dòng**, mỗi dòng một chữ cái. Ví dụ:

```
D-01 A
D-02 A
D-03 AB
D-04 A
D-05 A
D-06 A
D-07 A
D-08 A
```

Không trả lời dòng nào ⇒ dòng đó dùng **mặc định** (cột cuối bảng 7.2). Nếu không trả lời gì, đêm chạy **Plan A** ở 7.5.

### 7.2 Bảng quyết định

| Mã | Câu hỏi | Phương án | Mặc định |
|---|---|---|---|
| **D-01** | Ngưỡng flake của gate | **A** giữ gate mềm (chỉ báo), `--strict` chỉ dùng khi đo · **B** `--strict` + retry 1 lần · **C** chỉ chạy arch hay flake nhất | **A** |
| **D-02** | Có đóng băng tính năng (G2) đêm nay? | **A** đóng băng: chỉ sửa + đo · **B** cho phép làm tính năng song song · **C** chỉ cho phép **khảo sát thiết kế** (đọc/viết tài liệu, không code) | **A** |
| **D-03** | T26 đánh hướng nào | **A** `fsck` trước — ⚠️ *đã kiểm: `fsck` SẠCH ⇒ hết việc* · **B** đánh "ai chen giữa": thêm `client_id` + file/inode vào `VRING`, rồi đo X/Y · **C** làm thẳng hàng rào bền vững | **B** |
| **D-04** | `MAX_IO` dùng để đo đêm nay | **A** 256 (lộ 41 % bug, nhanh kết luận; **không** bật ở gate/phát hành) · **B** giữ 128 (xanh nhưng che bug) | **A** |
| **D-05** | Cụm teardown: vá bao nhiêu chỗ trong đêm | **A** tối đa **2 chỗ**, mỗi chỗ một vòng riêng, hồi quy ⇒ revert · **B** tối đa 4 chỗ · **C** không đụng, chỉ ghi tài liệu | **A** |
| **D-06** | Có chạy "câu cá" soak dài trên `ps` không | **A** có, ~600–1000 boot x86_64 nền (săn T-EXEC/T31/T-ROOT/T-SCHED) · **B** không, dồn hết cho T26 | **A** |
| **D-07** | Nợ kiến trúc (tách `lib.rs` 17k dòng) | **A** không đụng đêm nay · **B** tách sau khi T26 xong trong đêm · **C** tách ngay từ đầu đêm | **A** |
| **D-08** | Đồ hoạ / ViFastKey | **A** không đụng · **B** chỉ viết tài liệu thiết kế (0 dòng code sản phẩm) | **A** |

Ghi chú cho từng quyết định:

* **D-01** nghiêng A vì flake nền ~1 %/boot mà gate chỉ chạy 9 boot ⇒ `--strict` sẽ đỏ oan **~8,6 %** số lượt. B sẽ che lỗi thật; C bỏ sót arch.
* **D-02** nghiêng A vì T26 là **mất dữ liệu**, và `MAX_IO` đang bị hạ để che nó.
* **D-03** ⚠️ **đã cập nhật sau kiểm r295.1:** mặc định đổi từ **AB → B**. Vì `fsck=0` là **sạch** và `vend` là **hệ quả**, nhánh (a) không còn việc để làm ⇒ chạy hai nhánh song song là tốn công. Nay là **B** (đánh "ai chen giữa") và làm **3 kiểm 0-boot trước**: (1) `fsck` sạch/hỏng, (2) `vend` có phải hệ quả, (3) `req=0` là client nào. (c) chỉ sau khi hiểu cơ chế.
* **D-04** nghiêng A: 128 chỉ là che; 256 cho **41 %** tái hiện, tức 20 boot là kết luận.
* **D-05** nghiêng A vì `reap_task` gộp đã hỏng 0/3 cả ba arch — bài học "đo từng chỗ".
* **D-06** nghiêng A vì soak chạy nền **không tranh CPU với phân tích** (máy `ps` khác máy A), và đây là cách duy nhất gặp lại các lớp 1 %.
* **D-07** nghiêng A: đụng `lib.rs` đúng lúc đang sửa FS là tự gây hồi quy.
* **D-08** nghiêng A: chưa có yêu cầu tính năng, và protocol tối giản hiện đủ cho ViFastKey (đã chốt user-space).

### 7.3 Ngân sách đêm (giả định cửa sổ 8 giờ)

| Khoản | Thời gian | Ghi chú |
|---|---:|---|
| Chốt cây + baseline (gate 10/10, 3×3) | 0:25 | bắt buộc, làm trước mọi thứ |
| T26 — phân tích + vá + đo | 3:30 | trọng tâm |
| Teardown (D-05) | 1:30 | mỗi chỗ ~45 phút |
| Câu cá soak nền trên `ps` (D-06) | 0 (nền) | chạy song song, không chiếm giờ người |
| Đo mở rộng + CI | 1:00 | chốt số cho báo cáo |
| **Đóng băng code + viết tài liệu** | 0:45 | **bắt buộc, không thương lượng** |
| Gate + 3×3 chốt + ký SHA-256 | 0:30 | trên HEAD cuối |
| Dự phòng | 0:20 | cho build hỏng / revert |

**Nếu cửa sổ khác 8 giờ:** giữ nguyên 0:45 đóng băng + 0:30 chốt; phần T26 co giãn theo phần còn lại, và **luôn** giữ baseline 0:25 ở đầu.

### 7.4 Lịch chạy (mốc giờ tính từ lúc bắt đầu, không phải giờ đồng hồ)

| Mốc | Việc | Cổng kiểm |
|---|---|---|
| 0:00–0:25 | `git status` sạch hoá phần dở; commit kết quả r295 (đang treo); ghi `BUILD_ID`; gate 10/10; 3×3 QEMU | gate xanh + `RETRY 0/9` |
| 0:25–0:30 | Bật soak nền trên `ps` nếu D-06 = A (600 boot x86_64, `VIFAST_RETRY_TIMEOUT=0`, `KEEP_SERIAL=1`) | soak chạy, log riêng |
| 0:30–1:00 | **T26 kiểm 0-boot** (mục 4.0): xác nhận `fsck` sạch, `vend` là hệ quả, `req=0` là root/pager | ba kết luận có/không, ghi ngay |
| 1:00–2:15 | **T26 — ai chen giữa**: thêm `client_id` (badge) + file/inode vào bản ghi `VRING`; in ring trong khoảng `mkdir → VEND idx=6`; đo **20 boot** trên `ps` (~80 s) | biết task nào chen, op gì, đụng block nào |
| 2:15–3:15 | **Phép đo phân định X/Y** nếu ring đủ dữ kiện: X = hoãn đường đọc của pager tới hết V2 (**chỉ đo, không phải bản vá**; cảnh báo deadlock ở 4.1); Y = `stress-fs` song song root exec; mỗi lượt 20 boot | X về 0/20 ⇒ xác nhận client xen giữa |
| 3:15–3:45 | Nếu đã có cơ chế: viết **test đỏ-trước trên host** (hai client xen đúng thứ tự ring), rồi mới vá; đo lại 20 boot | tối đa **3 vòng** giả thuyết; hồi quy ⇒ revert |
| 3:45–4:15 | Đọc kết quả soak nền; phân loại ca đỏ; đếm `USER FAULT` cho T31 | bảng phân loại có số |
| 4:15–5:45 | Teardown theo D-05: từng chỗ, 3 arch × 3 boot; hồi quy ⇒ revert ngay | 3/3 xanh cả 3 arch mới giữ |
| 5:45–6:30 | **Đo mở rộng**: T26 40 boot × 2 máy để có CI hẹp; hoặc soak 400 boot nếu T26 chưa có cơ chế | mỗi số kèm N, máy, `BUILD_ID` |
| 6:30–7:15 | **ĐÓNG BĂNG CODE.** Chỉ viết `progress-g1.md` + báo cáo + bảng số + CI | không sửa mã nguồn nữa |
| 7:15–7:45 | Gate 10/10 + 3×3 trên HEAD cuối; ký SHA-256 báo cáo | xanh, hoặc ghi rõ đỏ vì lớp nào |
| 7:45–8:00 | Viết "3 việc tiếp theo" + "cần agent0 quyết gì" | bàn giao |

### 7.5 Plan A — chạy nguyên văn nếu không ai trả lời

1. **Đóng băng tính năng** (D-02 A), giữ `--strict` tắt (D-01 A), không đụng nợ kiến trúc (D-07 A), không đụng đồ hoạ (D-08 A).
2. Đo T26 ở `MAX_IO=256` **chỉ trong nhánh đo** (D-04 A); **không** đổi cấu hình gate/phát hành.
3. T26 đánh theo **D-03 B**: làm **3 kiểm 0-boot** trước (mục 4.0), rồi thêm `client_id` + file/inode vào `VRING`, rồi mới đo X/Y. Hàng rào bền vững (c) **chỉ** sau khi có cơ chế.
4. Teardown tối đa **2 chỗ** (D-05 A), bắt đầu bằng pager server; mỗi chỗ một vòng riêng; hồi quy ⇒ revert, **không** thử lại đêm nay.
5. Soak nền 600 boot x86_64 trên `ps` (D-06 A) để săn T-EXEC / T31 / T-ROOT / T-SCHED.
6. 6:30 đóng băng code tuyệt đối; sau đó chỉ tài liệu.

### 7.6 Luật tự quyết (để không phải hỏi lúc 3 giờ sáng)

**Được phép tự quyết:**
* Thêm **đèn/marker chẩn đoán** (chỉ in, không đổi hành vi), miễn sau đó gate 10/10.
* **Revert** bản vá của chính mình khi có hồi quy (D18).
* Đổi feature **chỉ-dùng-đo** (`stress-fs`) và cờ môi trường đo.
* Chạy soak dài, chia nhỏ, chạy lại.
* Commit tài liệu + kết quả đo.

**KHÔNG được phép (kể cả khi thấy "dễ sửa"):**
* Nới ngưỡng hay sửa test/accept() để xanh.
* Bật `--strict` mặc định.
* Đổi cấu hình phát hành, bật `MAX_IO=256` ở gate/phát hành.
* Nâng `MAX_TCBS` / `MAX_OBJECTS` / `KERNEL_STATIC_BUDGET`.
* Gộp nhiều chỗ teardown vào một bản vá.
* Xoá marker đang dùng để đo.
* Sửa `vifast-fs` theo hướng "chặn hết" (khoá thô) mà không có test đỏ-trước.

### 7.7 Điều kiện dừng & báo động

| Tình huống | Hành động |
|---|---|
| Bản vá gây hồi quy 3 arch | **Revert ngay**, ghi vào nhật ký, **không** thử lại ý đó trong đêm |
| Đến mốc **4:30** mà T26 chưa có cơ chế | Dừng đọc code, chuyển sang **đo mở rộng + viết tài liệu** (kết quả âm cũng là kết quả) |
| Build hỏng > 15 phút | Revert về commit cuối, build lại, xác nhận 3×3, tiếp tục việc khác |
| Gate đỏ do lớp `T-INFRA` | Chạy lại tối đa **2 lần**, ghi nhận, **không** sửa cho xanh |
| Xuất hiện chữ ký mới | Lưu artifact, **không** đuổi nếu < 3 lần xuất hiện |
| QEMU treo | Timeout 400 s, kill, đánh dấu, tiếp |
| SSH/`ps` rớt | Soak phải **ghi artifact từng boot** để chạy lại phần thiếu; không làm lại từ đầu |
| Đến **6:30** | **Đóng băng code**, chỉ viết tài liệu |

### 7.8 Sản phẩm bàn giao lúc 6:00–8:00

1. `docs/reviews/progress-g1.md`: một mục cho **từng khối giờ** (kể cả khối thất bại, kèm số).
2. **Bảng số** thống nhất: `cấu hình | máy | N | số đỏ | CI 95 % | BUILD_ID`.
3. HEAD sạch, không file dở; **gate 10/10**; **3×3 QEMU** trên HEAD cuối.
4. **Một trang kết luận**: "T26 = (cơ chế hoặc chưa biết)", "đã bác thêm gì", "3 việc tiếp theo", "cần agent0 quyết gì".
5. Báo cáo ký **SHA-256**; artifact các ca đỏ nằm trong `artifacts/`.

### 7.9 Rủi ro có thể đốt cả đêm, và cách chặn

| Rủi ro | Cách chặn (đã có sẵn) |
|---|---|
| Đo nhầm binary cũ | `BUILD_ID` in trong serial, đối chiếu trước khi tin số |
| Tưởng "hết bug" vì `0/N` | luật Part 3: `0/N` chỉ là trần, phải ghi CI |
| Đuổi theo chữ ký mới giữa đêm | ngưỡng ≥3 lần mới đuổi; dưới thì chỉ lưu artifact |
| Sửa gộp rồi hỏng cả 3 arch | luật D-05/D18: một chỗ một vòng, hồi quy revert |
| Gate đỏ vì flake rồi "sửa cho xanh" | luật 7.6: cấm nới ngưỡng; chạy lại tối đa 2 lần |
| Hết giờ mà không có báo cáo | mốc 6:30 đóng băng code là **bắt buộc** |
| Máy `ps` mất kết nối giữa soak | artifact từng boot, chạy bù phần thiếu |

---

## Part 8 — Số liệu thô để tự kiểm

**Mốc thời gian (11/11):** `ba0cab8` 11:46 → `d451ea9` 11:34 (vá `tcb_wait`) → `1439354` 12:05 (revert `reap_task`) → `3d4ac89` 12:15 (mẫu số sạch `2/120`) → `53a0d0f` 12:27 (tổng kết vòng 20–40) → `4efdbcd` 12:40 (đính chính `0/301 → 1/301`) → **r295 chưa commit** (13 file, `+1098/−68`, build 14:06 OK).

**Bảng flake tích luỹ (mỗi dòng một cấu hình riêng — KHÔNG cộng dồn):**

| Cấu hình | N | Hỏng |
|---|---:|---:|
| x86_64 nền (trước LINT0) | 305 | 8 (2,6 %) |
| sau LINT0 | 301 | 2 (0,66 %) |
| sau `KERNEL_READY` | 301 | 3 (1,0 %) |
| `ps`, sau vá `tcb_wait` | 120 | **2 (1,7 %)** — cả hai `EXEC FAIL` |
| `ps`, sau vá `tcb_wait` | 80 | 1 |
| riscv64, sau vá T33b | 150 | 0 |
| riscv64, sau vá T33b | 201 | 1 (`IPC RIGHTS OK`) |
| riscv64 nền (trước vá) | 200 | 2 (panic hart phụ) |
| soak G1 (3 arch × 100) | 300 | 2 (99,3 %) |
| **r295, `MAX_IO=256`, máy `ps`** | **22** | **9 (41 %) — T26** |
| r295, cùng cấu hình, máy A | 10 | 0 |
| r295, vòng lặp FS trong boot | 22.000 phép thử | **0** |

**Chi phí đo (dùng để lập lịch):**

| Số boot | Thời gian ước tính |
|---:|---:|
| 9 (3 arch × 3) | ~35 s |
| 20 | ~80 s |
| 40 | ~2,5 phút |
| 120 | ~7,5 phút |
| 600 | ~37 phút |
| 1.000 | ~62 phút |

**Artifact đối chiếu:** `artifacts/ps-r295-S1n/` (20 boot, 7 đỏ), `artifacts/ps-r295-S1r/` (2 boot, 2 đỏ), `artifacts/r295-S1|S2/` (máy A, 10 xanh).

**Một dòng serial ca đỏ (đã lược):**
```
VEND seq=31 idx=6
V2RD end n=06
DIRP s=0512 b=1 e0=06 mm=00
VRING srv=150 req=0 op=8194 idx=0 res=0 h=1744276933
V2FAIL v2=1 fsck=0
VSTAT vseq=0 varg=0 vhash=0 vend=4
```

---

## Phụ lục — Từ điển nhanh

| Thuật ngữ | Nghĩa |
|---|---|
| **marker** | chuỗi guest in ra serial để harness biết bước nào đã chạy |
| **flake** | lỗi chỉ xuất hiện một tỉ lệ lần boot, cùng một binary |
| **`T26`, `T31`, `T33b`…** | tên nội bộ của từng lớp lỗi |
| **`vend` / `VSEQ` / `VARG` / `VHASH`** | đèn chẩn đoán r295: số lần `readdir` trả ENOENT / reply sai seq / tham số sai / payload bị đè |
| **`fsck`** | hàm kiểm tra toàn vẹn FS sau khi ghi |
| **`V2`** | khối kiểm thử FS end-to-end lúc boot (tạo, đọc, liệt kê, xoá) |
| **`MAX_IO`** | kích thước đệm IPC cho FS (128 hay 256) |
| **`reap_task`** | helper dọn task (suspend → wait → destroy) đã thử và **hỏng cả 3 arch** |
| **`stress-fs`** | feature chẩn đoán: lặp 1000 vòng FS trong một lần boot (không bật ở gate) |
| **CI 95 %** | khoảng tin cậy 95 % của tỉ lệ lỗi — dùng để biết một con số nhỏ có ý nghĩa hay không |
