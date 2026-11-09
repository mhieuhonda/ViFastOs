# Bộ cấp trang tự do & uỷ quyền cap cho exec (G2)

Trạng thái: **proposal** — soạn ở r263 (T23 của `ViFastOs-decision-r254.md`).
Chưa có dòng mã nào; tài liệu này chốt *hợp đồng* để G2 chia việc.
Nguồn sự thật khi triển khai: `crates/vifast-kernel/src/lib.rs` (Untyped,
retype), `crates/vifast-abi/src/lib.rs` (`user_layout`, `vfs`), và
`crates/vifast-client/src/main.rs` (shell).

## 0. Bối cảnh đo được (r259)

| Số đo | Giá trị | Nguồn |
|---|---|---|
| Trang Untyped dữ liệu đã dùng | `77/96` (x86_64), `77/96` (riscv64) | `DATA UNTYPED pages=…` trên serial |
| Object cấp từ Untyped | `66/86` | `DATA UNTYPED objects=…` |
| Trang client ELF | 6 / 7 / 7 (riscv64 / aarch64 / x86_64) | `check_client_budget` |
| Ngân sách trang client | 8 / 8 / 9 | `scripts/build.sh` |
| Một lần exec (ước lượng) | ~5 trang | chú thích `scripts/build.sh` vòng 206 |
| Bộ cấp trang tự do | **chưa có** | chỉ có Untyped + `retype` |

Hệ quả của việc "chưa có": mọi frame/TCB/VSpace đều phải `retype` từ một
Untyped mà root giữ. Root phải tự chia ngân sách theo *dự đoán* nhu cầu của
chương trình con, và mỗi lần exec lại tiêu thêm trang. Đó là lý do tồn tại
`check_client_budget` — một hàng rào ở build time thay cho một bộ cấp phát
ở run time.

## 1. Phần A — bộ cấp trang tự do

### A.1 Hai lựa chọn

| | A-kernel: allocator trong hạt nhân | A-server: memory server ở user space |
|---|---|---|
| Ai giữ danh sách trang trống | hạt nhân | một TCB riêng, giữ cap Untyped |
| Cấp phát | syscall mới (`PageAlloc`) hoặc `retype` tự động | IPC tới memory server |
| Kiểm chứng | host test trực tiếp | phải chạy QEMU, khó test đơn vị |
| Độ phức tạp hạt nhân | + (mảng free list, khoá) | ~0 (dùng lại Untyped) |
| Tính "microkernel" | giảm (chính sách trong kernel) | giữ nguyên (chính sách ở user) |
| Trần hiện tại | Untyped 96 trang | Untyped 96 trang |

### A.2 Đề xuất: A-server, nhưng **đo trước**

Lý do chọn A-server:

1. Hạt nhân hiện **không có** khái niệm trang tự do; thêm vào là thêm chính
   sách cấp phát vào đúng chỗ D-decisions muốn giữ nhỏ.
2. Đã có sẵn cơ chế đúng nghĩa: Untyped + `retype` + `revoke`. Memory server
   chỉ là *một* TCB giữ Untyped và trả lời IPC — không cần syscall mới.
3. Mọi thứ cần đo (số trang còn lại, ai giữ bao nhiêu) trở thành dữ liệu
   user space: `free`/`df` đọc được, test được bằng chương trình thật.

Nhưng **chưa triển khai ngay**: cần một phép đo trước để biết có đáng không.

### A.3 Phép đo bắt buộc trước khi viết mã (D17)

Trả lời được ba câu bằng số, nếu không thì dừng:

1. **Exec tiêu bao nhiêu trang thật?** Đếm `DATA UNTYPED pages` trước và sau
   một `runfs`, lặp 5 lần; kỳ vọng ~5 trang/lần, sai số do frame IPC dùng lại.
2. **Bao nhiêu lần exec đồng thời là tối đa hiện tại?** Tăng dần `runfs` cho
   tới `MEM OOM OK`; ghi lại ngưỡng.
3. **Nếu chỉ thêm free-list trong root (không cần server) thì được bao nhiêu
   lần exec?** Đây là phương án A0 rẻ nhất: root tự giữ một mảng trang đã
   `retype` sẵn và cấp lại bằng `revoke` + `retype`, không cần TCB mới.

Chỉ khi (3) không đủ thì mới làm A-server.

### A.4 Hợp đồng nếu đi A-server

| Việc | Ai làm | Ghi chú |
|---|---|---|
| Giữ Untyped | memory server | root chuyển cap lúc boot |
| Cấp frame | `retype(Untyped → Frame)` trong server | không đổi hạt nhân |
| Trả frame | `revoke` từ cap đã cấp | đường đã có (K69) |
| Hỏi còn bao nhiêu | IPC `STAT` | `free`/`df` dùng |
| Ai được cấp | theo badge của người gửi | xem Phần B |

Điều **không** làm: cấp trang theo địa chỉ vật lý tuyệt đối, hay để kernel
tự chọn frame khi user hỏi "cho tôi một trang". Cả hai đều phá mô hình cap.

## 2. Phần B — uỷ quyền cap trong ABI exec

### B.1 Vấn đề: `df` là builtin vì nó cần cap

`df` hiện nằm trong shell (`crates/vifast-client/src/main.rs`, nhánh
`line == b"df"`) và gọi `call_message(1, vfs::STATFS, …)` — tức nó dùng
**endpoint VFS của chính shell** (slot 1). Chương trình con exec từ FS thì
không có cap đó.

**ĐÍNH CHÍNH (r267)** — bản đầu của tài liệu này viết "exec không có đường
truyền cap", và đó là **sai**. Đường đó đã có sẵn:

- `CapMint(src_slot, dst_tcb_slot, dst_slot, rights, badge)`: caller cần WRITE
  trên TCB đích; cap nguồn phải **chưa có badge** thì mới được đặt badge mới.
  Root dùng đúng lời gọi này để cấp cho client slot 1 (badge 1) và slot 6
  (badge 2) ngay lúc boot (`crates/vifast-root/src/main.rs`, khối `cap_mint`
  cho `client_tcb`) — tức cơ chế "cấp cap cho task khác" **đã được dùng thật**.
- `CapCopy`: bản sao không đổi badge (root dùng để đưa endpoint cho server).

Vậy thứ thiếu **không phải primitive** mà là hai việc nhỏ:

1. `spawn_elf_from_fs` không `cap_mint` cho chương trình con (chỉ thiếu 3 dòng).
2. **Chính sách badge**: con thừa hưởng badge nào (badge 1 = danh tính shell,
   hay badge riêng)? Badge là `uid` mà FS server dùng để kiểm quyền file, nên
   đây là quyết định bảo mật chứ không phải chi tiết kỹ thuật.

`TcbGrantCap` (r264) vẫn có chỗ đứng riêng: nó là bản **không đặc quyền** —
uỷ quyền một cap ĐÃ CÓ badge mà không cần mint badge mới, dùng khi người uỷ
quyền không phải root. Nhưng nó **không phải** điều kiện tiên quyết để đưa
`df` ra applet.

### B.2 Hợp đồng đề xuất: cap đi qua CNode của chương trình con

| Bước | Syscall | Nội dung |
|---|---|---|
| 1 | `TcbConfigure` | như hiện tại |
| 2 | `TcbSetArgs` (D15) | `argc/argv` như hiện tại |
| 3 | **`TcbGrantCap`** (đã có từ r264) | `(tcb_slot, src_slot, dst_slot, rights)` — chuyển cap từ caller sang cspace của con |
| 4 | `TcbStart` | chạy |

Ràng buộc bắt buộc của `TcbGrantCap` (mỗi cái là một test):

1. Chỉ chuyển được cap mà root **thật sự có**, đúng kiểu, đủ quyền.
2. Quyền bị **thu hẹp** khi chuyển: con không bao giờ nhận `GRANT` (đúng
   nguyên tắc đã áp cho IPC, test `cap_transfer_requires_grant_and_strips_the_right`).
3. `dst_slot` phải trống; chuyển vào slot đã có ⇒ `SlotOccupied`, không ghi đè.
4. Chuyển xong, root **giữ** cap (không phải move) trừ khi có cờ `MOVE`.
5. Destroy TCB con ⇒ cap đã cấp bị thu (đường `remove_caps_to` đã có).

### B.3 `df` nhận đúng cái nó cần, không hơn

| Cap | Quyền | Vì sao |
|---|---|---|
| endpoint VFS | `READ` (chỉ gửi request, không nhận cap) | `STATFS` chỉ đọc |
| badge | badge của shell | server dùng badge làm `uid` để kiểm quyền file |

Badge là điểm mấu chốt: `vfs_*_via(slot, …)` hiện đã đi qua slot 6 với badge
khác (đường `perm` chứng minh cách ly), nên hạ tầng badge **đã có**; thiếu
chỉ là đường truyền cap lúc exec.

Hệ quả cần nói rõ: `df` chạy bằng badge của shell nên **thấy đúng phần nó
được thấy**. Nếu sau này muốn `df` thấy toàn bộ FS, đó là quyết định riêng
(badge mới hoặc quyền `STATFS` toàn cục) — không được làm ngầm.

### B.3bis Trạng thái triển khai (r264, đính chính r267)

> Đọc kèm đính chính ở B.1: primitive đã có (`CapMint`), thiếu chỉ là wiring +
> chính sách badge. `TcbGrantCap` là bản không đặc quyền, không phải blocker.

`SyscallId::TcbGrantCap = 57` đã có trong kernel với 5 ràng buộc ở B.2, mỗi
ràng buộc một host test:

| Ràng buộc | Test |
|---|---|
| caller phải có WRITE trên TCB đích | `grant_cap_syscall_requires_write_on_the_target_tcb` |
| slot đích phải trống | `grant_cap_rejects_occupied_destination` |
| quyền bị thu hẹp (không bao giờ GRANT) | `grant_cap_strips_grant_right` (đỏ khi bỏ clamp) |
| badge giữ nguyên từ cap nguồn | `grant_cap_keeps_the_source_badge` |
| không chuyển Untyped gốc | `grant_cap_cannot_hand_out_the_root_untyped` |
| derive, caller không mất cap | `grant_cap_installs_a_derived_cap_in_the_target_cspace` |

**Còn lại của Phần B**: nối nó vào đường `exec` (root cấp endpoint VFS + badge
cho chương trình con) rồi chuyển `df` ra applet. Việc này cần chốt **chính sách
badge** (badge nào để con thừa hưởng — của shell hay của root), đúng câu hỏi 4
ở C.3, nên chưa làm trong đêm nay.

### B.4 Điều kiện "xong" của Phần B

- `df` chạy được từ `/bin/vf` (không còn builtin), in cùng con số như bản
  builtin trong cùng một lần boot.
- Bỏ cap của `df` ⇒ `df` báo lỗi quyền, **không** panic, và shell vẫn sống.
- Có test host cho `TcbGrantCap` (5 ràng buộc ở B.2, mỗi ràng buộc một test
  đỏ-trước theo D18).
- Client ELF **không tăng trang** sau khi bỏ builtin `df` (đo lại
  `check_client_budget`; dự kiến giảm vì bớt một nhánh).

## 3. Phần C — thứ tự, rủi ro, câu hỏi mở

### C.1 Thứ tự triển khai (mỗi bước một vòng, gate xanh mới sang bước sau)

| Bước | Việc | Vì sao trước |
|---|---|---|
| 1 | Đo A.3 (ba câu) | quyết định A0 hay A-server bằng số, không bằng cảm giác |
| 2 | `TcbGrantCap` + 5 test đỏ-trước | độc lập với bộ cấp trang |
| 3 | Chuyển `df` ra `/bin/vf` dùng cap được cấp | chứng minh Phần B end-to-end bằng một lệnh thật |
| 4 | A0: root giữ free-list, cấp lại bằng `revoke`+`retype` | rẻ nhất, không TCB mới |
| 5 | (chỉ khi bước 4 không đủ) A-server | đắt nhất, làm sau cùng |

### C.2 Rủi ro đã biết

| Rủi ro | Dấu hiệu | Cách chặn |
|---|---|---|
| Cap rò sang chương trình con | con giữ cap sau khi chết | test destroy + `check_invariants` (đã có) |
| Quyền bị nới khi chuyển | con có `GRANT` | test ràng buộc 2 ở B.2 |
| Exec hết trang giữa đường | `MEM OOM OK` xuất hiện sớm | đo A.3 trước, giữ ngân sách client |
| Free-list ẩn làm sai `DATA LEAKED` | `DATA LEAKED ≠ 0` | chạy `soak` sau mỗi bước |
| Badge bị giả | `BADGE SPOOF DENIED OK` mất | giữ marker này trong gate |

### C.3 Câu hỏi mở (cần Agent 0 hoặc người thiết kế chốt)

1. **Ai được cấp trang?** Chỉ root, hay mọi TCB có cap Untyped? (Nghiêng:
   chỉ theo cap — ai có cap thì được, không cần luật riêng.)
2. **`TcbGrantCap` có phải syscall mới, hay mở rộng `TcbConfigure`?** Syscall
   riêng rõ ràng hơn, và giữ được thứ tự "configure → args → caps → start".
3. **Trần cap cho chương trình con** là bao nhiêu? Hiện `MAX_CAPS_TASK` là
   hằng số cố định; nếu mỗi applet cần 2–3 cap thì cần biết trần thật.
4. **`df` có nên là applet đầu tiên, hay chọn lệnh đọc-only khác?** `df` là
   ứng viên tốt nhất vì chỉ cần `STATFS` + `READ`.
5. **Có giữ builtin `df` làm đường dự phòng** trong một vòng chuyển tiếp
   không? (Nghiêng: không — hai đường cùng tồn tại sẽ che lỗi cap.)

## 4. Tóm tắt một trang

- **Phần A**: chưa có bộ cấp trang tự do; đề xuất đo ba câu trước, rồi đi
  A0 (free-list trong root) — chỉ làm memory server nếu số đo buộc phải làm.
- **Phần B**: `df` phải ở lại shell vì exec không có kênh uỷ quyền cap; sửa
  bằng `TcbGrantCap` (thu hẹp quyền, không move, không ghi đè slot) và cấp
  cho `df` endpoint VFS quyền `READ` + badge shell.
- **Phần C**: 5 bước, mỗi bước một vòng; 5 rủi ro có dấu hiệu và cách chặn;
  5 câu hỏi mở cần chốt trước khi viết mã.
