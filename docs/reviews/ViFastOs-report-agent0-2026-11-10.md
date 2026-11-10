# ViFastOs — Báo cáo tiến độ & tồn đọng

**Bản dành cho Agent 0** (người đọc không có quyền truy cập mã nguồn)

| | |
|---|---|
| Mốc HEAD | `1355f01` — r288, 10/11/2026 |
| Cây làm việc | **đang dở r289** (10 file sửa + 1 crate mới chưa commit) |
| Người lập | phiên soát độc lập, không sửa mã nguồn |
| Quy mô | 25 crate Rust, ~46 000 dòng, 3 kiến trúc CPU |
| Tài liệu kèm theo | `docs/audits/2026-11-10-soat-tang-day.md` (bản kỹ thuật, có số dòng) |

---

## 0. Cách đọc báo cáo này

Báo cáo được viết để **tự đứng một mình**: mọi kết luận đều kèm mô tả bằng lời,
trích đoạn mã đủ ngắn để đọc tại chỗ, và kịch bản gây lỗi. Bạn không cần mở file
nào trong dự án.

**Ba mức tin cậy** dùng xuyên suốt:

| Nhãn | Nghĩa |
|---|---|
| **[T]** | Phiên này tự đọc mã, hoặc tự chạy lệnh và thấy kết quả |
| **[S]** | Một trong 5 mũi soát phụ đã **tái hiện được** bằng chương trình nháp ngoài repo; tôi đọc lại đoạn mã liên quan nhưng chưa tự chạy lại |
| **[Đ]** | Suy ra từ đọc mã, **chưa có đường tái hiện** — cần kiểm trước khi tin |

Mọi mục P0/P1 đều có cấu trúc: **chuyện gì → vì sao sai → kịch bản → vì sao
lưới test hiện tại không bắt → đề xuất**. Mục 8.C có lệnh tái hiện cho người
có máy.

---

## 1. Tóm tắt điều hành

### 1.1 Trạng thái một câu

ViFastOs đã là một **microkernel kiểu seL4 chạy thật trên 3 kiến trúc**, có
capability, IPC, bộ nhớ ảo, SMP 2 nhân, filesystem có journal và shell tương
tác — nhưng **tầng đáy nhất (platform/arch) là tầng duy nhất không có test
host, và cả 3 lỗi nghiêm trọng nhất của đợt soát này đều nằm ở đó** (2 lỗi)
hoặc ở một bất biến của tầng capability (1 lỗi).

### 1.2 Ba việc cần bạn quyết (chi tiết ở mục 7)

1. **Có cho phép "đóng băng tính năng" 4 vòng để làm sạch tầng đáy không?**
   Hiện mỗi vòng vẫn xen tính năng mới (T27, T32, T33…), nên nợ cấu trúc tích
   thêm nhanh hơn tốc độ dọn.
2. **Ai sửa gốc của hồi quy `FS V2 OK` trên x86_64?** Đây là **BLOCKER còn mở
   duy nhất** của một lỗi đã biết từ r262: `main` chỉ xanh vì đã hạ một hằng số
   (`MAX_IO` 256 → 128) để thu hẹp cửa sổ race, **không phải sửa gốc**.
3. **Có chấp nhận "hợp đồng tầng" (mỗi tầng phải có một phép kiểm có thể đỏ)
   làm tiêu chí hoàn thành G2 không?** Nếu có, tầng platform phải có mô hình
   chạy trên host trước khi mở thêm tính năng.

### 1.3 Sáu con số chốt

| Số | Giá trị | Ý nghĩa |
|---|---|---|
| Syscall | **58/58 có handler thật** | không còn placeholder |
| Test host | **344** (kernel 243 + 2 fuzz `#[ignore]`) | chạy trong gate, không cần QEMU |
| Case âm của harness | **632** | mỗi marker boot có luật exact-one |
| QEMU gate | **3 kiến trúc × 3 lần** | RISC-V, AArch64, x86_64 (UEFI) |
| Soak | **299/300 = 99,7 %** (r281) | 100 lần boot × 3 arch |
| Lỗi P0 mới | **3** | đều là lỗi *cách ly*, không phải lỗi chức năng |

---

## 2. Dự án là gì (không cần đọc mã)

### 2.1 Ý tưởng

Một hạt nhân (kernel) **làm rất ít**: giữ chìa khóa quyền (capability), xếp
lịch, chia bộ nhớ ảo, chuyển thư (IPC). Mọi thứ khác — filesystem, driver,
shell — chạy ở user mode như các tiến trình riêng. Hỏng một tiến trình không
sập cả hệ thống. Đây là mô hình **seL4**, viết bằng Rust, không dùng heap.

Ba đặc điểm quyết định mọi thứ còn lại:

1. **Không heap.** Toàn bộ object nằm trong một mảng tĩnh 168 phần tử
   (`MAX_OBJECTS`). Cấp phát = "nặn" (retype) một cục bộ nhớ thô (Untyped)
   thành object. Hết chỗ là lỗi trả về, không phải panic — nhưng cũng không
   có cách nào "xin thêm".
2. **Một lõi, ba bản build.** Cùng thư viện `vifast-kernel`, ba binary cho
   RISC-V / ARM / x86. Không có binary nào chạy cả ba.
3. **Capability = chìa khóa.** Muốn làm gì cũng phải có chìa đúng quyền trong
   không gian tên của mình. Chìa chỉ được **giảm** quyền khi copy, không được
   tăng. Đây là nền của toàn bộ an ninh hệ thống.

### 2.2 Năm tầng

| Tầng | Trách nhiệm | `unsafe` | Test host |
|---|---|---|---|
| `vifast-abi` | Hằng số chung: số syscall, mã lỗi, quyền, layout bộ nhớ user | cấm | 8 |
| `vifast-kernel` | Object, capability, IPC, scheduler, bộ nhớ ảo (portable) | **cấm** | **243** |
| `vifast-runtime` | Boot, dispatch syscall, timer/preemption (dùng chung 3 arch) | **cấm** | 0 |
| `vifast-platform-*` | Trap entry, MMU, timer, IRQ, SMP — **nơi duy nhất có asm/MMIO** | có | **0** |
| `vifast-arch-*` | UART, đồng hồ, mặt nạ ngắt, tắt máy | có | 7 |

Cộng thêm các crate "phía trên" (root task, VFS server, shell/client,
filesystem, virtio, ACPI, ELF loader, page-table encoder).

**Điểm mấu chốt của báo cáo này:** tầng thứ tư — tầng đáy nhất, nơi mọi thứ
phần cứng thật sự xảy ra — là tầng **duy nhất có 0 test host**. Nó chỉ được
kiểm gián tiếp qua marker in ra serial khi boot trong QEMU.

### 2.3 Các giới hạn cứng (agent0 cần biết vì nhiều lỗi là "hết chỗ")

| Giới hạn | Giá trị | Hệ quả |
|---|---|---|
| Object tối đa | 168 | mọi thứ (TCB, Frame, VSpace…) tranh nhau một mảng |
| TCB tối đa | 24 | mỗi hart 4 hàng đợi, mỗi hàng 24 chỗ |
| Hart tối đa | 4 | hiện chỉ khởi động được 2 |
| Cap | root 128 / task 16 | task thường rất chật |
| CNode | phẳng, 16 slot | không phân cấp |
| Mapping / VSpace | 64 | **trần cứng**: ELF > ~60 trang không exec được |
| fd / tiến trình | 8 | |
| `Kernel` (bộ nhớ tĩnh) | ≤ 566 272 byte | có `assert` lúc biên dịch |
| Ảnh đĩa | 8192 block × 512 B = **4 MiB** | 64 inode, tên 28 B, path 128 B |

---

## 3. Tiến độ

### 3.1 Các giai đoạn

| Giai đoạn | Nội dung | Trạng thái |
|---|---|---|
| G0 | Boot 3 kiến trúc, vào user mode, IPC, capability, VSpace | **XONG** |
| G1 "Nền vững" | Sửa 44 mục audit K32–K75; SMP 2 nhân; bất biến scheduler ở ranh giới user; đóng 0 OPEN | **XONG** (r263) |
| G2 "Lên cao" | Đưa applet ra ngoài (`/bin/vf`), uỷ quyền cap cho `exec`, pager, hart 2–3 | **ĐANG LÀM** (~40 %) |

### 3.2 Bảng treo việc

**Đã xong và có bằng chứng đo được**

- Boot + vào user mode + timer preempt trên RISC-V, AArch64, x86_64/UEFI.
- 58/58 syscall có handler thật; `CapMint`/`CapRevoke`/`TcbGrantCap` chạy end-to-end.
- SMP 2 nhân trên cả 3 arch; mỗi hart một hàng đợi + work stealing; TLB shootdown.
- Filesystem v3 có journal, mất điện giữa chừng vẫn replay được; hard link,
  symlink, double indirect; test persistence 7 phase.
- Shell tương tác thật (đọc một luồng serial, gõ từng lệnh chờ marker).
- Applet đầu tiên `/bin/df` chạy bằng capability được uỷ quyền (r268) — chứng
  minh đường `exec` + cấp chìa đã khép kín.
- Bất biến scheduler ở ranh giới user: ~167 000 phép kiểm trên RISC-V,
  ~95 000 trên x86, **0 vi phạm** (r259).

**Đang làm dở (trong cây, chưa commit)**

- r289: vá P0-1 (bit `nG` cho AArch64) và P0-2 (hộp thư maintenance dùng
  chung) — xem mục 4.

**Chưa làm**

- Gốc của race `FS V2 OK` trên x86 (BLOCKER, xem 3.5).
- Hart 2–3 (cần fan-out IPI; hiện `SECONDARY_ONLINE` sẽ sai — P1-8).
- Mạng đầy đủ (chỉ có ARP/UDP/DNS mức demo), màn hình (chỉ splash), driver
  ngoài QEMU.
- Trần 64 mapping / 168 object: chưa có thiết kế "cấp trang động".

### 3.3 Hạ tầng kiểm chứng — tài sản lớn nhất của dự án

Đây là phần agent0 nên đánh giá cao nhất, vì nó quyết định *có thể tin số đo
hay không*:

| Cơ chế | Nội dung | Vì sao quan trọng |
|---|---|---|
| **Gate 8 bước** | build → test kernel → đếm số test → fuzz → QEMU 3×3 → persistence → harness → docs+fmt | một lệnh, đỏ là chặn |
| **Marker exact-one** | mỗi bước boot in một chuỗi (vd `FS V2 OK`); harness đòi **đúng một** lần | bắt được cả "không chạy" và "chạy hai lần" |
| **632 case âm** | mỗi luật marker có test chứng minh nó *thật sự* bắt được sai | chống "test rỗng" |
| **D18 — test đỏ trước** | mỗi bản vá phải kèm test đỏ trên mã cũ, xanh trên mã mới | chống vá mò |
| **Mutation** | cố ý làm hỏng một bất biến, gate **phải** đỏ | chứng minh phép kiểm không vô dụng |
| **Soak** | 100 lần boot × 3 arch | bắt flake 1 % |
| **Fuzz syscall** | 100 000 bước ngẫu nhiên × 3 định dạng bảng trang | bắt bất biến bị phá |

### 3.4 Số đo hiện tại

| Hạng mục | Số | Ghi chú |
|---|---|---|
| Dòng Rust | ~46 000 | 25 crate |
| Test host | 344 | kernel 243 (+2 fuzz `#[ignore]`), fs 40, elf 19, pte 12, abi 8, acpi 8, arch 7, virtio 7 |
| Test trong gate | 243 + 2 fuzz | runtime/user/fsutil **0 test** — kiểm qua QEMU |
| Cảnh báo clippy | **80** | 0 lỗi; phần lớn là style, nhưng có 4 assert hằng vô nghĩa |
| Soak gần nhất | 299/300 | r281, 10/11 |
| Sàn test trong gate | **138** | ⚠ thực tế 245 — xoá 107 test vẫn xanh |

### 3.5 Các sự cố lớn đã đóng (để thấy *kiểu* lỗi của dự án)

| Sự cố | Triệu chứng | Gốc | Bài học |
|---|---|---|---|
| `FS V2 OK` (r262) | 25,5 % số lần boot x86 thiếu marker | race trong đường FS server khi ≥2 hart; hạ `MAX_IO` chỉ thu hẹp cửa sổ | **BLOCKER vẫn mở** — xem 5.4 |
| T32 (r271→r273) | soak 100 treo sau `shell>` | bản vá "cho huỷ TCB đã Exited" gây hồi quy; phải revert rồi áp lại | đo trên 1 arch không đủ |
| T33/T33a (r275→r288) | treo ~1 % trước `PREEMPT OK` | `has_ready_on` không kiểm `state == Ready` ⇒ hart phụ thức dậy, không có việc, tự `exit(1)` | hai hàm phải đồng ý với nhau thì phải có test đối chiếu |
| K32–K75 (r260) | 44 mục audit | đủ loại: queue, cap, IRQ, shootdown | đóng được vì có mô hình host |

**Nhận xét pattern:** gần như mọi lỗi nặng đều thuộc một trong hai họ —
(a) **hai nơi phải đồng ý với nhau nhưng không có test đối chiếu**, hoặc
(b) **một bất biến chỉ đúng nhờ may mắn về thứ tự** (BKL, thứ tự publish, thứ
tự khởi động hart). Ba lỗi P0 mới đều thuộc họ (b).

---

## 4. Trạng thái cây làm việc ngay lúc viết báo cáo

> ⚠ **Đọc mục này trước khi giao việc.** Cây không ở trạng thái "sạch đã kiểm".

- **HEAD** = `1355f01` r288 — đã commit bản vá T33a/T33b (hart phụ hết việc
  không còn giết cả máy). Bản vá này đúng hướng và đã có test đỏ-trước.
- **Chưa commit (r289, đang làm dở):** 10 file sửa + 1 crate mới
  `crates/vifast-maint/` (183 dòng). Nội dung đúng là hai việc tôi đề xuất:
  1. `vifast-pte`: thêm bit `nG` (bit 11) cho mọi leaf user AArch64 + test
     đỏ-trước `aarch64_leaf_is_non_global`. **Đã có trong cây.**
  2. `vifast-maint`: hộp thư maintenance dùng chung, publish `kind` **trước**
     `seq`, consumer **không ACK khi chưa có việc** — thay cho 3 bản sao sai
     giống nhau ở 3 port. Có 3 test host, trong đó 1 test hai luồng 20 000 lần.
  3. 3 platform crate chuyển sang dùng `vifast-maint`; `flush_va` AArch64 nay
     gắn ASID (nếu không, việc thêm `nG` sẽ làm nó thành no-op).
- **Trạng thái kiểm (tôi đã chạy):** `cargo test -p vifast-pte` xanh (12/12);
  `cargo check` cho **cả ba** target bare-metal (riscv64, aarch64, x86_64) đều
  `exit=0`. **Gate QEMU 3 arch chưa chạy trên trạng thái này** — biên dịch sạch
  không có nghĩa là chạy đúng ở tầng asm/IPI.
- **Rủi ro cần biết:** đây là refactor tầng đáy, chạm đúng chỗ không có test
  host. Nếu gate QEMU không xanh sau khi commit, phải coi là hồi quy của
  r289, không phải flake.

---

## 5. Tồn đọng kỹ thuật

### 5.0 Bảng tổng hợp

| Mức | Số | Bản chất |
|---|---|---|
| **P0** — phá cách ly / an toàn bộ nhớ | **3** | TLB toàn cục, race ACK, cap lạc |
| **P1** — chức năng / bền vững, với tới được | **10** | 5 của filesystem, 5 của kernel/platform |
| **P2/P3** — tiềm ẩn, nợ nhỏ | ~18 | phần lớn là "im lặng bỏ qua" và "kiểm rỗng" |
| **Nợ cấu trúc** | 4 khoản | 1 file 16 328 dòng, dispatch 1 401 dòng, 3 port trùng 73 %, harness trong đường nóng |

---

### 5.1 [P0-1] AArch64: mọi trang user là "toàn cục" ⇒ đổi tiến trình không xoá TLB

**Chuyện gì.** Trên ARM, mỗi mô tả trang (PTE) có một bit tên `nG`
(*non-global*). Nếu bit này **không** được bật, entry TLB được coi là *global*:
nó **không** thuộc về không gian địa chỉ (ASID) nào, và lệnh xoá TLB theo ASID
(`tlbi aside1is`) **không** xoá nó. Đúng theo kiến trúc ARM.

Mã hiện tại ghép cờ cho leaf như sau (trước r289):

```
(pa & PA_MASK) | 0b11 | AF | SH_INNER | ap | PXN | uxn
```

Không có bit 11 (`nG`). Trong khi đó đường đổi không gian địa chỉ lại dùng:

```
msr ttbr0_el1, <root | asid<<48>
tlbi aside1is, <asid>        ← chỉ xoá entry của ASID này
```

**Vì sao sai.** Hai điều trên không hợp nhau: hệ thống *tin* rằng xoá theo
ASID là đủ, nhưng mọi entry của nó lại là global nên không bị xoá.

**Kịch bản.** Hệ này có ít nhất ba không gian địa chỉ cùng lúc: của root,
của VFS server, của client. **Cả ba đều map chương trình ở cùng một địa chỉ
ảo `CODE_VA`, nhưng vào các khung nhớ vật lý khác nhau.** Chuyển từ root sang
client: entry `CODE_VA → khung của root` là global nên sống sót; client nhảy
vào `CODE_VA` và **chạy mã của root**.

**Vì sao hôm nay vẫn xanh.** Hai lý do, cả hai đều là may mắn:
1. Đường boot cho root và server **dùng chung một khung** ở `CODE_VA` (hàm
   `build_task_space_shared_code`), nên entry global tình cờ đúng cho cả hai.
2. QEMU có thể xoá TLB rộng tay hơn phần cứng thật.

Đây là lỗi **"xanh trên QEMU, sai trên máy thật"** đúng nghĩa — loại lỗi nguy
hiểm nhất vì mọi số đo hiện có đều nói "tốt".

**Vì sao lưới test không bắt.** Không có test host nào cho tầng platform; test
của `vifast-pte` chỉ kiểm quyền (AP/UXN/PXN), không kiểm bit `nG`.

**Đề xuất.** `| (1 << 11)` + test đỏ-trước (đã có trong r289). **Bắt buộc
kèm:** `flush_va` phải gắn ASID, nếu không nó thành no-op sau khi thêm `nG`.
Đã có trong r289.

---

### 5.2 [P0-2] Shootdown TLB: publish `seq` trước payload ⇒ mất một lần xoá

**Bối cảnh.** Khi một hart sửa bảng trang, các hart khác có thể còn giữ entry
TLB cũ. Cơ chế chuẩn: gửi một "hộp thư" (mailbox) — publisher ghi yêu cầu
vào hộp thư của hart đích, bắn ngắt, rồi chờ ACK. Ba hart chạy song song nên
hộp thư phải có giao thức đúng.

**Chuyện gì (cả 3 port đều sai giống nhau).**

```
publish:                              consume:
  seq = SEQ.fetch_add(1) + 1            seq = SEQ.load()
  KIND.fetch_or(kind)                   if ACK >= seq: return
  doorbell()                            kind = KIND.swap(0)
                                        if kind & TLB: flush()
                                        ACK.store(seq)     ← VÔ ĐIỀU KIỆN
```

**Vì sao sai.** Có một khe thời gian giữa hai lệnh publish. Hart đích poll
đúng khe đó sẽ thấy `seq` mới nhưng `kind == 0`; nó làm **không gì cả** rồi
vẫn `ACK.store(seq)`. Publisher thấy đủ ACK và tin rằng TLB đã được xoá. Ngắt
gửi sau đó bị chặn bởi `ACK >= seq`. ⇒ **một lần invalidate bị mất vĩnh viễn.**

**Vì sao khe đó không hiếm.** Trên x86, hart phụ poll chính trong vòng quay
chờ khóa lớn (BKL) — đúng lúc hart chính đang giữ BKL để làm flush. Trên ARM
và RISC-V cũng vậy. Tức là **xác suất gặp khe này tỉ lệ với mức tranh chấp
khóa**, chứ không phải một xác suất cực nhỏ.

**Hậu quả.** Entry TLB cũ trỏ vào khung nhớ đã được trả về Untyped và cấp lại
cho tiến trình khác ⇒ **đọc/ghi chéo vùng nhớ giữa hai tiến trình**. Đây là
lỗi cách ly, cùng họ với P0-1 nhưng xảy ra trên cả ba kiến trúc.

**Vì sao lưới test không bắt.** Có mutation test `test-skip-tlb-ipi` (cố ý bỏ
IPI) nhưng nó kiểm *đường đi*, không kiểm *thứ tự*. Race chỉ hiện dưới tải.

**Đề xuất.** Đảo thứ tự: ghi `kind` (Release) **trước**, tăng `seq` (Release)
sau, rồi rào chắn + doorbell; consumer chỉ ACK khi đã thật sự lấy được việc.
r289 đã làm đúng việc này trong crate dùng chung `vifast-maint`, kèm test hai
luồng 20 000 lần và một test "mô phỏng publisher cũ" phải đỏ.

**⚠ Câu hỏi còn mở cho agent0.** Hộp thư mới chỉ chứa `seq/kind/ack`. **Payload
của lệnh xoá (địa chỉ ảo, ASID, số trang) vẫn nằm ở mảng riêng của từng
port.** Nếu hai lần xoá khác địa chỉ được publish trước khi consumer kịp lấy,
mảng payload bị ghi đè ⇒ một lần xoá dùng sai địa chỉ. Hôm nay điều này không
xảy ra **chỉ vì** mọi publish đều nằm trong vùng khóa BKL. Đó là bất biến
ngầm, không được phát biểu, không có test. Nên đưa payload vào chính hộp thư
(hoặc ghi rõ bất biến "chỉ publish khi giữ BKL" và kiểm bằng `assert`).

---

### 5.3 [P0-3] Chìa khóa lạc: `cap.parent` trỏ TCB đã chết ⇒ `revoke` không tới

**Bối cảnh.** Mỗi chìa khóa (cap) có một con trỏ tới chìa "cha" (nơi nó được
mint ra). `revoke` đi ngược chuỗi cha để tìm mọi chìa phái sinh. Nếu chuỗi cha
đứt, `revoke` **dừng và trả về 0** — coi như "không có gì để thu hồi".

**Chuyện gì.** Khi một TCB chết, hệ thống phải "tái neo" các con trỏ cha đang
trỏ vào chìa của TCB đó sang chìa sống khác. Hàm làm việc này
(`reparent_tcb_caps`) có hai lỗ hổng cộng dồn:

1. Nó **return sớm** nếu TCB đang dùng một CNode (bảng chìa riêng), vì "CNode
   là object độc lập, không thuộc TCB".
2. Nhưng vòng lặp sửa `cap.parent` chỉ đi qua các bảng chìa **với tới được từ
   TCB đang dùng**. Một CNode đã bị tháo khỏi TCB **không ai sửa**.

**Kịch bản [S] đã tái hiện.**

```
1. Mint một chìa vào CNode C, nguồn là slot 2 của TCB A
   ⇒ C.slots[2].parent = {tcb: A, slot: 2}
2. Huỷ A            ⇒ cap.parent bây giờ là con trỏ chết
3. revoke chìa gốc  ⇒ đi ngược: gặp bước nhảy không phân giải được ⇒ trả Ok(0)
                       ⇒ chìa phái sinh VẪN SỐNG
4. destroy object   ⇒ thấy "còn chìa trỏ tới" ⇒ HasDependents
                       ⇒ object KHÔNG BAO GIỜ bị huỷ được
5. check_invariants ⇒ Ok(())   ⇒ bộ kiểm bất biến MÙ với trường hợp này
```

**Vì sao sai.** Đây là vi phạm trực tiếp mô hình bảo mật: một chìa lẽ ra phải
bị thu hồi lại tiếp tục tồn tại, và object của nó trở thành **bất tử** (rò bộ
nhớ vĩnh viễn trong một hệ không có heap). Bộ kiểm bất biến không bắt được vì
nó chỉ kiểm `cap.parent` trong nhánh `badge != 0`.

**Vì sao lưới test không bắt.** Test hiện có (`cnode_caps_participate_in_delete_and_revoke`)
giữ CNode **vẫn gắn** vào TCB và chỉ khẳng định "thu hồi được ≥ 1 chìa" — đúng
con số mà lỗi này tạo ra.

**Đề xuất.** Không vá riêng lỗi này. Gốc là **có bốn bộ duyệt bảng chìa độc
lập** trong kernel (mỗi bộ tự quyết định "bảng nào là bảng thật"). Gộp thành
**một bộ duyệt cấu trúc duy nhất** trên mọi bảng chìa vật lý (root, TCB,
**mọi** CNode dù đã tháo hay chưa), rồi viết lại 4 chỗ dùng nó. Đây là việc
của một vòng, và nó dọn luôn 2 lỗi P2 đi kèm (đếm trùng, kiểm rỗng).

---

### 5.4 [P1] Mười lỗi chức năng

| # | Lỗi | Hậu quả | Tin cậy |
|---|---|---|---|
| P1-1 | FS: khi mount dùng **dung lượng thiết bị** thay vì dung lượng đã format; hàm kiểm bit thiếu chặn biên | **Panic** khi mount đĩa lớn hơn vùng đã format (4 MiB) | **[T] đã tái hiện** |
| P1-2 | FS: đọc bảng gián tiếp bằng chỉ số không kiểm | Ảnh FS độc hại ⇒ **halt kernel** thay vì trả `Corrupt` | [S] |
| P1-3 | FS: ghi quá EOF để lại lỗ, không zero-fill | File thành **không đọc được**, `fsck` báo hỏng. Client với tới được | [S] |
| P1-4 | FS: `create` thất bại để lại inode mồ côi đã stage | Lần commit sau ghi nó xuống đĩa ⇒ `fsck` hỏng | [S] |
| P1-5 | FS: `unlink` giải phóng inode **đang mở**, inode được tái dùng | fd cũ đọc/ghi **file khác** — rò dữ liệu | [S] |
| P1-6 | x86: hart phụ **không bao giờ** shootdown hart chính | Hart chính giữ TLB cũ ⇒ đọc khung đã tái cấp | [T] |
| P1-7 | Budget CPU cạn ⇒ hart tự `exit(1)` | Đường `Sleep` đã được vá T33a, đường budget thì chưa | [T] |
| P1-8 | "Số hart phụ đã online" dùng như **bitmask** | Chỉ đúng khi hart lên đúng thứ tự. Boot hart 2 mà hart 1 chết ⇒ publish vào hart không tồn tại | [T] |
| P1-9 | ACPI: đọc RSDP/MADT vượt biên với bảng ngắn | Firmware lạ ⇒ **panic lúc boot** | [T] |
| P1-10 | `reply_epoch` **không bao giờ tăng** | Guard chống "reply cũ sau restart" của K49 là `0 == 0` — **rỗng** | [T] |

**Chi tiết 5 lỗi nặng nhất:**

**P1-1 (đã tái hiện, có lệnh ở 8.C).** Định dạng một ảnh 4 MiB, sau đó làm
thiết bị lớn hơn (8 MiB) — chuyện bình thường khi phân vùng lớn hơn ảnh, hoặc
khi nâng dung lượng đĩa. Khi mount, vòng quét metadata chạy theo **dung lượng
thiết bị** (16 384 block) trong khi bitmap chỉ có 8 192 bit ⇒
`index out of bounds: the len is 1024 but the index is 1024`.

Nghiêm trọng vì: (a) nó xảy ra với **ảnh hợp lệ**, không cần dữ liệu độc;
(b) cùng một biến `capacity` còn dùng để tính `next_free`, nên hệ còn có thể
cấp phát ra ngoài vùng FS khai báo.

**P1-3/P1-4/P1-5 (FS, client với tới được).** Cả ba đều là lỗi *bền vững dữ
liệu*, loại khó phát hiện nhất khi chạy demo vì chúng không làm sai ngay:

- ghi vào offset 1 MiB của file rỗng ⇒ file đó **không đọc lại được** nữa;
- tạo file khi thư mục đầy ⇒ trả `NoSpace` nhưng để lại inode mồ côi, lần ghi
  sau commit nó xuống đĩa;
- `unlink` một file đang mở rồi tạo file mới ⇒ inode được cấp lại, fd cũ
  **đọc ra nội dung file mới** (rò dữ liệu giữa hai file, `fsck` không thấy).

**P1-6 (x86 bất đối xứng).** Trên x86, code shootdown chỉ cho phép hart phụ là
đích (`if target != 1 { continue }`) và chỉ hart chính mới được gửi lệnh xoá
toàn bộ. Nhưng hart phụ **có chạy syscall** và **có sửa bảng trang**. Vậy khi
hart phụ unmap một trang, hart chính không bao giờ được báo. Không có bộ đếm
nào ghi nhận đường này nên nó **vô hình** trong mọi số liệu.

**P1-10 (guard rỗng).** `reply_epoch` được khởi tạo 0 và **không có chỗ nào
tăng nó** (đã grep toàn repo). Mọi so sánh `reply_epoch == epoch` là `0 == 0`.
Nghĩa là cơ chế "reply của server cũ không được giao cho client mới" **không
hoạt động**; thứ duy nhất còn che là kiểm trạng thái `BlockedReply`. Đây là
kiểu lỗi nguy hiểm về *niềm tin*: một mục audit (K49) đã được đóng dựa trên
một guard không tồn tại.

---

### 5.5 [P2/P3] Mười tám khoản tiềm ẩn (nhóm theo bản chất)

**Nhóm A — "im lặng bỏ qua" (nguy hiểm vì không có triệu chứng):**

- `enqueue` bỏ rơi TCB Ready nếu mọi hàng đợi đầy — hiện **bất khả thi** vì
  `MAX_TCBS` = dung lượng hàng = 24, nhưng **không còn khe dự phòng**, và
  comment trong mã ghi "18" (số cũ, sai).
- Đăng ký trang staging quá 32 ⇒ bỏ, rò khung nhớ.
- `shootdown_va` có `if !BKL.is_held() { return; }` — mà `is_held()` trả
  **true cả khi đang chờ** khóa, không chỉ khi đang giữ. Guard này không còn
  chứng minh được điều nó tuyên bố.
- `StackSpec.guard` (trang bảo vệ stack) bị bỏ qua bằng `let _ = stack.guard;`.

**Nhóm B — "kiểm rỗng" (test xanh nhưng không kiểm gì):**

- `check_invariants` có một phép kiểm tự dựng lại tham chiếu rồi kiểm nó —
  không thể fail.
- Bốn `assert` trong test của `vifast-pte` so sánh **hằng số với hằng số**.
- `reply_epoch` (P1-10).
- Sàn đếm test trong gate: đòi **≥ 138** trong khi thực tế **245**.

**Nhóm C — tài nguyên không được canh:**

- Trap stack: **16 KiB** trên x86/AArch64 nhưng **32 KiB** trên RISC-V; nửa
  mảng không dùng; **không canary**. Tràn stack sẽ ghi đè stack hart bên cạnh,
  im lặng.
- Bộ đếm `sleepers_active`/`budgeters_active` rò khi huỷ TCB đang ngủ.
- `set_tcb_affinity` không kiểm TCB đang chạy ⇒ có thể đẩy nó ra khỏi hart.
- `wake_error` không gỡ TCB khỏi hàng chờ endpoint/notification.
- `wait_tcb` dùng chung trường `caller` với `Call` ⇒ con đang chờ bị trả
  `WouldBlock` sai.

**Nhóm D — biên số học / trần cứng:**

- `untyped_take`: hai phép cộng/trừ không `checked_*` (chỉ chạm khi địa chỉ
  sát `u64::MAX`).
- Trần **64 mapping** + 1 Frame/trang ⇒ ELF > ~60 trang **không exec được**.
- `build_task_space` chấp nhận ảnh rỗng ⇒ entry không map.

**Nhóm E — tầng driver (chưa từng bị soi vì không có test):**

- VirtIO block: không kiểm `used.len`/`used.id`; sau timeout dùng lại descriptor
  mà thiết bị có thể còn sở hữu; **DMA dùng địa chỉ kernel ảo**; không đọc
  `QueueNumMax`. Thiết bị lỗi ⇒ trả 512 byte rác như dữ liệu đĩa.
- x86: handler #DF lấy khóa BKL **trước** khi xem vector ⇒ nếu #DF xảy ra
  trong vùng tới hạn, chính handler panic vì "BKL re-entry" — mất khả năng
  chẩn đoán đúng lúc cần nhất.

**Nhóm F — mã chết:**

- Kernel có một ramfs nhỏ + 6 syscall `Open/Read/Write/Close/Stat/Seek` mà
  **không chương trình nào gọi** (đã grep cả 3 tiến trình user). Toàn bộ I/O
  thật đi qua VFS server bằng IPC. Đây là ~300 dòng + 6 mặt ABI chết.

---

### 5.6 Nợ cấu trúc — bốn khoản, có số đo

**Nợ 1: một file 16 348 dòng.** `vifast-kernel/src/lib.rs` = **7 463 dòng mã +
8 885 dòng test**. Một khối `impl Kernel` duy nhất, 128 phương thức công khai.
Hàm dài nhất `user_syscall_on` = **1 401 dòng** (20 % toàn bộ mã không-test),
chứa 58 syscall.

Tách được thành 7 module mà **không đổi API, không đổi hành vi** (mọi thứ đã
là phương thức trên cùng một struct):

| Module đích | Dòng | Nội dung |
|---|---|---|
| `syscall/` | 1 450 | 58 syscall |
| `vspace.rs` (+ `task_space.rs`) | 1 318 (+400) | map/unmap/remap/teardown |
| `sched.rs` | 1 099 | hàng đợi, sleep, budget, suspend/resume/restart |
| `untyped.rs` | 723 | retype, cấp phát, thu hồi |
| `cspace.rs` | 645 | mint/copy/revoke/delete |
| `destroy.rs` | 540 | huỷ object + kiểm "đang dùng" |
| `invariants.rs` | 492 | bộ kiểm bất biến (chỉ test gọi — nên `#[cfg(test)]`) |
| còn lại | ~1 000 | kiểu dữ liệu, bảng object, khởi tạo |

**Nợ 2: dispatch syscall bằng chuỗi `if`.** 58 nhánh dạng
`match n { n if n == SyscallId::Send as u64 => … }`. Hệ quả kép:
- **hiệu năng**: 58 phép so sánh cho mỗi syscall, không thể sinh bảng nhảy;
- **an toàn**: nhánh `_ => NotImplemented` làm trình biên dịch **không kiểm
  được tính đầy đủ**. Thêm một syscall mới mà quên handler ⇒ biên dịch im
  lặng, chạy mới trả `NotImplemented`.

**Nợ 3: ba port platform trùng nhau 73–75 %.** Ba file 3 239 / 2 107 / 2 145
dòng. Đo trên dòng đã chuẩn hoá: AArch64↔RISC-V trùng **1 349/1 852 = 73 %**
(chiều ngược 75 %); x86↔AArch64 50 %; x86↔RISC-V 66 %. **23 hàm cùng tên** ở
cả ba. Giao thức maintenance là bản sao ba lần — và **cả ba đều sai giống
nhau** (P0-2). Đây là minh chứng rõ nhất cho quy tắc: *một giao thức copy ba
lần thì ba lần cùng sai*.

**Nợ 4: harness test nằm trong đường nóng.** Trait `Platform` (bề mặt để port
một kiến trúc mới) có **62 phương thức**, trong đó **21 phương thức chỉ để in
marker chẩn đoán lúc boot**. `on_timer` — hàm chạy 100 lần/giây trên mọi hart —
có ~150 dòng `#[cfg(test-exit)]` in `PREEMPT OK`, `SMP IDLE STALL`… Đã được
cô lập bằng feature nên bản phát hành không tốn, nhưng người đọc đường nóng
phải đi qua nó.

**Kèm theo:** `vifast-fs` 3 633 dòng một module phẳng; tách được
`ondisk / cache / journal / fsck`, trong đó `fsck` nên nhận dữ liệu qua tham
số thay vì `&mut self` — sửa luôn lỗi "fsck kiểm RAM thay vì kiểm đĩa".

---

## 6. Đã kiểm và thấy đúng (đừng bắt soát lại)

Để bạn không phải yêu cầu kiểm lại từ đầu:

- **Toán địa chỉ trang** của cả ba định dạng (3 cấp RISC-V, 4 cấp x86/ARM):
  chỉ số, độ rộng bảng, rollback khi dựng không gian địa chỉ — đúng.
- **W^X**: số đếm "frame đang được map ghi" / "map chạy" được bộ kiểm bất biến
  **tính lại từ bảng ngược và so sánh**, nên lệch sẽ đỏ ngay. Đây là lý do các
  lỗi W^X không lọt.
- **ELF loader**: kiểm chồng lấn, đồng dư trang, W^X, entry nằm trong segment
  chạy được, giới hạn địa chỉ — đúng và có test.
- **`has_ready_on` ↔ `pick_on`** đã đồng bộ (bản vá T33a, r288) — đúng hướng.
- **Cấp phát Untyped**: mọi đường lỗi sau khi đã lấy byte đều hoàn lại; danh
  sách vùng trống có coalesce và tự chữa.
- **`destroy_object`** kiểm lại toàn bộ điều kiện thay vì tin hàm kiểm trước — đúng.
- **`reply`** khôi phục `caller` khi tra chìa lỗi (r242) — đúng, có test đỏ-trước.
- **Đường block của runtime**: kiểm capability + kiểm vùng nhớ user trước khi
  chạm đĩa — đúng.
- **Khóa lớn BKL**: ticket lock FIFO, có 7 test host, guard `#[must_use]`.
- **`vifast-kernel` và `vifast-runtime` không có một dòng `unsafe` nào**, và
  chỉ có 3 chỗ `unwrap` — cả 3 đều đã được chặn bằng kiểm `is_some()` trước đó.

---

## 7. Câu hỏi cần Agent 0 quyết

### Q1. Có đóng băng tính năng 4 vòng để dọn tầng đáy không?

**Bối cảnh.** Nợ cấu trúc (Nợ 1–4) làm mỗi thay đổi sau này đắt hơn. Nhưng
dự án đang chạy theo vòng có tính năng mới, nên dọn dẹp luôn thua tốc độ phát
sinh nợ.

| Phương án | Ưu | Nhược |
|---|---|---|
| **A. Đóng băng 4 vòng** (r290–r293): chỉ sửa lỗi + tách module | tầng đáy sạch trước khi lên G2 phần lớn | chậm 4 vòng tính năng |
| B. Xen kẽ (1 vòng sửa / 1 vòng tính năng) | không chậm | nợ vẫn tăng, ngữ cảnh bị chia |
| C. Làm tiếp tính năng, dọn sau | nhanh trước mắt | mỗi lỗi P0 sau này đắt hơn, và refactor 16k dòng sẽ khó hơn nhiều |

**Khuyến nghị của tôi: A.** Lý do: 3 lỗi P0 hiện tại đều ở tầng không có test
host; muốn sửa chúng *một lần cho cả 3 arch* thì phải có crate dùng chung
(r289 đang làm đúng hướng). Nếu vừa sửa vừa thêm tính năng, mỗi vòng lại phải
kiểm lại 3 arch bằng QEMU — chậm hơn nhiều so với làm gọn một lần.

### Q2. Ai sửa gốc race `FS V2 OK` (BLOCKER còn mở)?

**Sự thật đã đo:** `MAX_IO = 256` ⇒ 12/47 lần boot x86 hỏng (25,5 %);
`MAX_IO = 128` ⇒ 0/32 hỏng. Xác suất 32 lần xanh chỉ do may mắn là ~0,01 %.
Triệu chứng: `mkdir` trả OK nhưng `readdir` ngay sau đó đọc **ảnh cũ** của
block thư mục gốc — vi phạm *read-your-own-write* ở tầng FS server, **chỉ khi
có ≥2 hart** (1 hart: 8/8 xanh).

Cần chốt: phiên kernel sửa, hay nhánh web (tác giả commit gây hồi quy) sửa
rồi phiên kernel kiểm lại. **Không được để hai bên cùng sửa.**

### Q3. "Hợp đồng tầng" có thành tiêu chí hoàn thành G2 không?

Đề nghị: mỗi tầng phải có **ít nhất một phép kiểm có thể đỏ** chạy trên host
(không cần QEMU). Hiện trạng: kernel ✅, fs ✅, runtime ⚠, platform ❌, arch ✅.

Nếu đồng ý, việc đầu tiên của tầng platform là **mô hình hộp thư + TLB chạy
trên host** (r289 đã mở đường bằng `vifast-maint`).

### Q4. Trần 64 mapping / 168 object: chấp nhận hay thiết kế lại?

Hiện một ELF > ~60 trang **không thể exec**. Đây là trần *kiến trúc*, không
phải bug. G2 có cần chạy chương trình lớn hơn không? Nếu có thì phải làm "cấp
trang động" **trước** khi làm thêm applet.

### Q5. Hart 2–3 có nằm trong tiêu chí G2?

Nếu có, phải sửa P1-8 (`SECONDARY_ONLINE` đang là bộ đếm dùng như bitmask) và
làm fan-out IPI **trước**. Nếu không, nên ghi rõ "G2 chỉ 2 hart" để tránh
người sau tưởng đã hỗ trợ 4.

### Q6. Có xoá phần mã chết (ramfs trong kernel + 6 syscall) không?

Ưu: giảm ~300 dòng + 6 mặt ABI, bớt một nguồn nhầm lẫn ("FS nào là thật?").
Nhược: có thể có kế hoạch dùng cho việc khác. Cần bạn xác nhận.

### Q7. Chấp nhận "hạ tầng đo" là sản phẩm chính thức không?

Hiện harness/marker/gate chiếm một phần đáng kể công sức. Nếu coi chúng là
sản phẩm (được test, được tài liệu hoá, được version) thì nên tách ra crate
riêng và bỏ khỏi đường nóng. Nếu coi là công cụ tạm thì nên nói rõ để không
ai trông cậy vào tính ổn định của marker.

---

## 8. Phụ lục

### 8.A Thuật ngữ

| Thuật ngữ | Nghĩa trong báo cáo |
|---|---|
| **Capability (cap)** | "Chìa khóa" cho phép làm một việc với một object. Có quyền (đọc/ghi/chạy/cấp lại), có badge (danh tính) |
| **Untyped / retype** | Bộ nhớ thô chưa định hình; "nặn" nó thành object gọi là retype |
| **VSpace** | Không gian địa chỉ ảo của một tiến trình |
| **Frame** | Một khung nhớ vật lý (4 KiB) |
| **PTE** | Phần tử bảng trang — mô tả một ánh xạ ảo→vật lý + quyền |
| **TLB** | Bộ đệm ánh xạ của CPU; phải được "xoá" khi bảng trang đổi |
| **ASID / PCID** | Nhãn không gian địa chỉ trong TLB, để đổi tiến trình không phải xoá sạch TLB |
| **nG (non-global)** | Bit trên PTE ARM: entry thuộc về một ASID, không dùng chung |
| **Shootdown** | Gửi ngắt cho các hart khác bảo chúng xoá TLB |
| **IPC** | Chuyển thư giữa tiến trình (Send/Recv/Call/Reply) |
| **Notification** | Cơ chế "bấm chuông" một chiều, dùng cho ngắt |
| **BKL** | Khóa lớn duy nhất của kernel — mọi hart phải giành khi vào kernel |
| **Marker** | Chuỗi in ra serial để harness biết một bước đã chạy đúng |
| **D18** | Quy tắc: bản vá phải kèm test đỏ trên mã cũ |
| **P0/P1/P2** | Mức lỗi: phá cách ly / sai chức năng / tiềm ẩn |

### 8.B Bản đồ thành phần (để nhờ người khác chạy lệnh)

| Cần xem gì | Ở đâu |
|---|---|
| Lõi kernel (object/cap/IPC/sched/VM) | `crates/vifast-kernel/src/lib.rs` (16 348 dòng) + `object.rs`, `ipc.rs` |
| Boot + dispatch syscall | `crates/vifast-runtime/src/lib.rs` (1 784 dòng) |
| Ba port phần cứng | `crates/vifast-platform-{riscv64,aarch64,x86_64}/src/platform.rs` |
| Hộp thư maintenance mới | `crates/vifast-maint/src/lib.rs` (183 dòng, **chưa commit**) |
| Mã hoá bảng trang | `crates/vifast-pte/src/lib.rs` |
| Filesystem + journal | `crates/vifast-fs/src/lib.rs` (3 633 dòng) |
| Cổng chất lượng | `scripts/gate.sh`, `scripts/test-qemu.py`, `scripts/test-harness-spec.py` |
| Báo cáo kỹ thuật của đợt này | `docs/audits/2026-11-10-soat-tang-day.md` |
| Tồn đọng audit cũ | `docs/audits/open-items.md` (0 OPEN tại r260) |
| Nhật ký quyết định | `docs/decisions/log.md`, `docs/reviews/` |

### 8.C Lệnh tái hiện (cho người có máy)

```sh
# 1) P1-1: panic khi mount đĩa lớn hơn vùng đã format
cd /tmp && dd if=/dev/zero of=big.img bs=512 count=8192
<repo>/target/debug/vifast-fsutil mkfs big.img
truncate -s 8M big.img
<repo>/target/debug/vifast-fsutil stat big.img /motd
#   → panicked: index out of bounds: the len is 1024 but the index is 1024
#     (crates/vifast-fs/src/lib.rs:1143)

# 2) Sàn test của gate đang lỏng
export PATH="$HOME/.cargo/bin:$PATH"
cd <repo> && cargo test -p vifast-kernel -- --list | grep -c ': test$'
#   → 245, trong khi scripts/gate.sh chỉ đòi >= 138

# 3) Trạng thái cây làm việc
cd <repo> && git log -1 --oneline && git status --short
```

### 8.D Đối chiếu phát hiện ↔ trạng thái

| Phát hiện | Trạng thái tại thời điểm viết |
|---|---|
| P0-1 (nG AArch64) | **đã vá trong cây r289**, chưa commit, chưa qua gate |
| P0-2 (thứ tự publish) | **đã vá trong cây r289** (crate `vifast-maint`), chưa commit |
| P0-3 (cap.parent lạc) | chưa ai chạm |
| P1-1…P1-5 (filesystem) | chưa ai chạm |
| P1-6 (x86 bất đối xứng) | chưa ai chạm |
| P1-7…P1-10 | chưa ai chạm |
| P2/P3 | chưa ai chạm |
| Nợ cấu trúc 1–4 | chưa ai chạm (r289 mới là bước đầu của Nợ 3) |

---

*Hết báo cáo. Nếu cần bản có số dòng chính xác cho từng mục, xem
`docs/audits/2026-11-10-soat-tang-day.md`.*
