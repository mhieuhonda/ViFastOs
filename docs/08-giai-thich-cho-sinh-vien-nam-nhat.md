# ViFastOs cho sinh viên năm nhất

Tài liệu này giải thích **các từ viết tắt và khái niệm** đang dùng trong ViFastOs
theo cách gần gũi nhất có thể. Mục tiêu không phải biến bạn thành chuyên gia
kernel sau một lần đọc, mà là giúp bạn:

1. Đọc được README và các tài liệu trong `docs/` mà không bị “đứng hình” vì
   thuật ngữ.
2. Hiểu mỗi thành phần trong dự án làm gì, vì sao nó tồn tại.
3. Biết thuật ngữ nào là **khái niệm chung của hệ điều hành**, thuật ngữ nào là
   **cách ViFastOs chọn triển khai**.
4. Phân biệt được “đã chạy được” với “đã hoàn chỉnh”.

> **Một lưu ý quan trọng về tài liệu:** code là nguồn sự thật cuối cùng. Các
> tài liệu `docs/01`–`docs/07` là thiết kế tham khảo từ mô hình Python ban đầu;
> một số chi tiết trong đó chưa được port sang bản bare-metal. Trạng thái hiện
> tại nên đọc `docs/00-hien-trang.md`, `docs/11-hien-tai/` và
> `ViFastOs-architecture-summary.txt`. Khi tài liệu nói một đằng mà code làm
> một nẻo, hãy tin code và cập nhật tài liệu.

---

## 0. Tóm tắt ViFastOs trong một phút

ViFastOs là một **hệ điều hành** viết bằng Rust, đi theo hướng **microkernel**,
chịu ảnh hưởng từ **seL4** — một microkernel nổi tiếng vì được kiểm chứng hình
thức rất mạnh. Nó không phải một bản Linux thu nhỏ. (Trong tên dự án,
**OS** = Operating System; “ViFast” là tên riêng, không phải từ viết tắt học
thuật.) Kernel của nó cố tình làm rất ít:

- quản lý **object** và **capability** (ai được làm gì với cái gì);
- **IPC** (các tiến trình gửi thông điệp cho nhau);
- **lập lịch** (thread nào được chạy CPU, khi nào);
- **bộ nhớ ảo** (page table, TLB);
- biến **ngắt** thành thông báo cho driver.

Mọi thứ còn lại — driver, filesystem, mạng, giao diện, bộ gõ — được thiết kế
để chạy ở **user space** như những tiến trình bình thường, chỉ có thêm
capability. Hiện dự án đã boot được trên QEMU với ba kiến trúc: **RISC-V 64**,
**AArch64** (ARM 64-bit) và **x86_64**. Nó đã có user mode, IPC, capability,
bộ nhớ ảo, filesystem ở user space, block driver, SMP cơ bản và nhiều test.
Nhưng nó vẫn là **nguyên mẫu nghiên cứu/giáo dục**, chưa phải hệ điều hành
dùng hằng ngày.

Ba câu để nhớ:

- **Kernel = người gác cổng + người điều phối**, không phải người làm mọi việc.
- **Capability = tấm vé không thể làm giả**, ghi rõ “được làm gì trên tài nguyên nào”.
- **IPC = cách các tiến trình nói chuyện**, là trung tâm của microkernel.

---

## 1. Nền tảng: hệ điều hành, kernel và quyền

### 1.1. OS, kernel, user space, kernel space

| Từ | Nghĩa đầy đủ | Giải thích cho người mới |
|---|---|---|
| **OS** | Operating System | Hệ điều hành. Lớp phần mềm nằm giữa ứng dụng và phần cứng: quản lý tiến trình, bộ nhớ, file, thiết bị. |
| **Kernel** | — | Lõi của OS, chạy với đặc quyền cao nhất. Muốn đọc file, gửi dữ liệu hay tạo tiến trình, ứng dụng phải nhờ kernel qua **syscall**. |
| **User space** | — | Vùng chạy của ứng dụng và dịch vụ, quyền hạn bị giới hạn. Driver, filesystem, mạng trong ViFastOs được thiết kế để chạy ở đây. |
| **Kernel space** | — | Vùng chạy của kernel, có quyền truy cập phần cứng và bộ nhớ đặc biệt. |
| **Syscall** | System call | Lời gọi hệ thống. Cửa chính thức để user space nhờ kernel làm việc. |
| **ABI** | Application Binary Interface | “Hợp đồng nhị phân” giữa hai phần mềm: thanh ghi nào chứa tham số, số hiệu syscall nào, cấu trúc dữ liệu xếp bao nhiêu byte. ABI khác **API** ở chỗ API là hợp đồng ở mức mã nguồn, ABI là hợp đồng ở mức mã máy. |
| **API** | Application Programming Interface | Tập hàm/kiểu dữ liệu mà một thư viện cung cấp cho lập trình viên. |
| **HAL** | Hardware Abstraction Layer | Lớp trừu tượng che giấu chi tiết phần cứng. Trong ViFastOs, các trait `Arch`/`Platform` đóng vai trò gần giống HAL. |
| **ISA** | Instruction Set Architecture | Tập lệnh mà CPU hiểu, ví dụ RISC-V, ARM64, x86-64. Cùng một chương trình C phải được biên dịch lại cho từng ISA. |

**Ví dụ dễ hiểu:** OS là một tòa nhà. Kernel là phòng bảo vệ + ban quản lý.
User space là các căn hộ. Muốn mở cửa kho, cư dân phải gửi yêu cầu xuống bảo vệ;
không được tự ý đập tường.

### 1.2. Microkernel và monolithic kernel

| Tiêu chí | Monolithic (ví dụ Linux) | Microkernel (ví dụ seL4, ViFastOs) |
|---|---|---|
| Thành phần trong kernel | Lập lịch, bộ nhớ, filesystem, mạng, driver… | Chỉ cơ chế tối thiểu: object/cap, IPC, lập lịch, bộ nhớ ảo, ngắt |
| Driver, FS, mạng | Chạy trong kernel space | Chạy ở user space như các server |
| Ưu điểm | Gọi hàm trực tiếp, nhanh, hệ sinh thái lớn | Kernel nhỏ, dễ kiểm chứng; một server chết có thể restart |
| Nhược điểm | Kernel lớn; một driver lỗi có thể kéo cả hệ thống | IPC tốn chi phí; nhiều server, debug khó hơn |
| TCB (trusted computing base) | Lớn | Nhỏ hơn |

**Microkernel không tự động nhanh hơn hay an toàn hơn.** Nó đổi một bộ
trade-off này lấy một bộ trade-off khác. ViFastOs chọn microkernel chủ yếu vì
mục tiêu học tập: kernel nhỏ, ranh giới quyền rõ ràng, IPC là bài học trung tâm.

### 1.3. TCB — một từ có hai nghĩa

Đây là chỗ rất dễ nhầm:

- **TCB = Thread Control Block**: object đại diện cho một thread trong kernel.
  Nó lưu trạng thái thread, entry point, stack, VSpace, danh sách capability,
  file descriptor, priority… Trong ViFastOs, khi tài liệu nói “TCB” thường là
  nghĩa này.
- **TCB = Trusted Computing Base**: tập hợp tất cả thành phần mà nếu chúng sai
  thì cả hệ thống mất tin cậy. Với microkernel, TCB nhỏ là một lợi thế.

Khi đọc, hãy nhìn ngữ cảnh: “TCB server/client” là thread; “thu nhỏ TCB” là
trusted computing base.

### 1.4. Quyền của CPU: ring, exception level, mode

Các kiến trúc gọi tên quyền khác nhau:

| Kiến trúc | Chế độ đặc quyền thấp (user) | Chế độ đặc quyền cao (kernel) |
|---|---|---|
| x86_64 | **ring 3** | **ring 0** |
| AArch64 | **EL0** (Exception Level 0) | **EL1** (EL2 dùng cho hypervisor, EL3 cho secure monitor) |
| RISC-V | **U-mode** (User) | **S-mode** (Supervisor); **M-mode** (Machine) cho firmware |

- **Ring** là vòng bảo vệ của x86. Số càng nhỏ quyền càng cao.
- **Exception level** là cấp đặc quyền của ARM.
- **M-mode/S-mode/U-mode** là ba mức của RISC-V.
- **Trap** là từ chung chỉ việc CPU chuyển từ user sang kernel vì một sự kiện.
  Trap gồm **interrupt** (ngắt từ thiết bị/timer), **exception** (lỗi như chia
  cho 0, page fault) và **syscall** (chủ động gọi kernel).
- **Vector table** là bảng địa chỉ các hàm xử lý trap/exception.
- **TrapFrame** là bản lưu thanh ghi của user trước khi vào kernel, để sau đó
  khôi phục và chạy tiếp.

### 1.5. Process, thread, context switch

| Từ | Nghĩa | Ghi chú |
|---|---|---|
| **Process** | Tiến trình | Có không gian địa chỉ riêng, tài nguyên riêng. |
| **Thread** | Luồng | Đơn vị chạy trong process; các thread cùng process chia sẻ bộ nhớ. |
| **Context** | Ngữ cảnh | Tập thanh ghi + PC + stack pointer… cần để chạy tiếp một thread. |
| **Context switch** | Chuyển ngữ cảnh | Lưu context của thread đang chạy, nạp context của thread khác. |
| **Scheduler** | Bộ lập lịch | Chọn thread nào chạy tiếp. |
| **Preemption** | Tiếm quyền | Kernel cưỡng bức dừng thread đang chạy để nhường CPU cho thread khác. |
| **Tick** | Nhịp timer | Mỗi lần timer ngắt. ViFastOs dùng 100 Hz (10 ms/tick) ở cấu hình test. |

### 1.6. Bản đồ crate trong repo

Một **crate** là một đơn vị code/package của Rust. Repo chia việc theo các crate
sau; đọc bảng này trước khi mở code sẽ dễ hơn nhiều:

| Crate | Vai trò | Ghi nhớ nhanh |
|---|---|---|
| `vifast-abi` | Hợp đồng nhị phân kernel/user | Số syscall, quyền, mã lỗi, `Message`, `BootInfo`. |
| `vifast-kernel` | Lõi portable | Object, capability, IPC, scheduler, VSpace, ramfs, invariant. Không biết địa chỉ UART. |
| `vifast-runtime` | Luồng boot/syscall/timer dùng chung | `Platform` trait, dispatch syscall, context switch. |
| `vifast-user` | Wrapper syscall cho user | Encode thanh ghi đúng cho từng kiến trúc. |
| `vifast-arch` | Trait/primitive phần cứng | Hợp đồng nhỏ giữa kernel và platform. |
| `vifast-pte` | Encode/decode PTE | Sv39, x86 4-level, AArch64 4 KiB. |
| `vifast-elf` | Parser ELF64 | Chỉ nhận `PT_LOAD`, kiểm tra W^X/range/overlap. |
| `vifast-root` | Root task Rust | Task user đầu tiên, tạo server/client/fault. |
| `vifast-server` | VFS server user-space | Nói chuyện IPC với client, gọi `vifast-fs`. |
| `vifast-client` | Client demo | Mở/đọc/ghi file, map frame, gọi IPC. |
| `vifast-fs` | Filesystem core | Superblock, inode, journal, hard link, symlink… |
| `vifast-fsutil` | Công cụ host | `mkfs`, `put`, `get`, `ls`, `fsck`, `inject-journal`. |
| `vifast-virtio` | Driver VirtIO-MMIO | Block device cho AArch64/RISC-V. |
| `vifast-acpi` | Parser ACPI | RSDP/XSDT/MADT cho x86. |
| `vifast-arch-*` | Primitive từng kiến trúc | UART, clock, IRQ mask, shutdown. |
| `vifast-platform-*` | BSP/trap/MMU/timer/SMP | Nơi duy nhất được dùng `asm!`, MMIO, port I/O. |
| `vifast-kernel-*` | Binary kernel từng target | Entry assembly, linker script, user image. |

Quy tắc kiến trúc quan trọng: **`vifast-kernel` và `vifast-runtime` là
`no_std` + `forbid(unsafe_code)`**. `unsafe` chỉ nằm ở platform/arch — nơi buộc
phải chạm thanh ghi, MMIO, assembly.

---

## 2. Quyền truy cập: object, capability, untyped

Đây là phần “linh hồn” của ViFastOs. Nếu chỉ đọc một phần, hãy đọc phần này.

### 2.1. Object là gì?

**Object** là một tài nguyên do kernel quản lý. Ví dụ:

- **TCB**: một thread.
- **Endpoint**: điểm hẹn để gửi/nhận IPC.
- **Notification**: một word bit dùng báo hiệu bất đồng bộ.
- **Frame**: một trang vật lý (4 KiB) có thể map vào không gian địa chỉ.
- **PageTable**: một trang bảng trang.
- **VSpace**: không gian địa chỉ ảo của một tiến trình.
- **Untyped**: vùng nhớ thô chưa được “định kiểu”.
- **CNode**: object chứa các slot capability (trong bản bare-metal hiện tại
  chưa triển khai đầy đủ).
- **SchedContext**: object chứa ngân sách CPU (chưa triển khai đầy đủ).
- **Iommu**: domain IOMMU phần mềm.

Kernel không cho user cầm con trỏ thô tới object. User chỉ cầm **capability** —
một tham chiếu có kiểm soát.

### 2.2. Capability (cap) — tấm vé không thể làm giả

**Capability** là một “vé” mà kernel phát hành. Nó cho biết:

- trỏ tới **object nào**;
- được làm gì trên object đó (**rights**);
- có **badge** gì (nhãn định danh — xem bên dưới);
- là **root** hay được derive từ cap khác;
- **parent** là ai (để thu hồi).

Các quyền cơ bản trong ViFastOs:

| Quyền | Bit | Ý nghĩa |
|---|---:|---|
| `READ` | 1 | Được đọc object/trang. |
| `WRITE` | 2 | Được ghi/sửa. |
| `EXEC` | 4 | Được thực thi (với trang code). |
| `GRANT` | 8 | Được phép chuyển capability này cho người khác qua IPC. |

Nguyên tắc vàng: **derive/copy chỉ được giảm quyền, không bao giờ tăng**.
Nếu bạn có vé chỉ đọc, bạn không thể tự biến nó thành vé ghi.

**Ví dụ:** object là một căn phòng. Capability là thẻ từ. Thẻ ghi “phòng 101,
được mở cửa, không được vào kho”. Bạn có thể photo thẻ cho đồng nghiệp với
quyền ít hơn, nhưng không thể tự thêm quyền vào thẻ.

### 2.3. CSpace, CNode, cptr, guard

- **CSpace** (Capability Space): “ví đựng vé” của một thread. Trong bản hiện
  tại, mỗi TCB có một mảng cố định các slot capability.
- **CNode** (Capability Node): object chứa một mảng slot. Nhiều CNode lồng nhau
  tạo thành CSpace nhiều cấp (đặc điểm của thiết kế seL4; bản bare-metal hiện
  tại mới dùng mảng phẳng).
- **cptr** (capability pointer): số 64-bit chỉ tới một slot trong CSpace.
- **Guard**: dãy bit phải khớp khi đi qua một CNode trong CSpace nhiều cấp.
  Guard giúp phân chia không gian tên capability giữa các thành phần.

**Ví dụ:** CSpace là một tủ có nhiều ngăn. cptr là số ngăn. CNode là một hộp
nhỏ đặt trong ngăn, bên trong lại có nhiều ngăn con. Guard là “mã khóa” để mở
đúng hộp.

### 2.4. CDT, derive, mint, copy, revoke

- **CDT** (Capability Derivation Tree): cây cha–con của các capability. Nếu bạn
  copy một cap, cap mới là con của cap gốc. Nhờ cây này, kernel biết phải thu
  hồi những cap nào.
- **Derive**: tạo cap mới từ cap cũ với quyền bằng hoặc ít hơn.
- **Mint**: tạo cap mới có **badge** hoặc quyền thay đổi (chỉ giảm quyền).
- **Copy**: sao chép cap sang TCB/slot khác, giữ quan hệ cha–con.
- **Revoke**: thu hồi toàn bộ cap con/cháu của một cap, giữ cap gốc. Đây là
  cách thu hồi quyền đã cấp mà không cần đi tìm từng người.
- **Delete**: xóa một cap cụ thể. Nếu cap còn con hoặc còn object, kernel có
  thể từ chối để tránh mất dấu tài nguyên.

**Ví dụ:** bạn là trưởng phòng, phát thẻ cho nhân viên. Khi một nhân viên nghỉ,
bạn `revoke` thẻ của họ; mọi thẻ copy từ thẻ đó cũng mất hiệu lực. Bạn không
phải đi từng phòng để thu hồi.

### 2.5. Badge — nhãn định danh người gửi

**Badge** là một số gắn vào capability endpoint/notification khi **mint**.
Người nhận thấy badge trong message, nhờ đó biết ai vừa gửi mà không cần tin
vào nội dung message.

Trong ViFastOs:

- Badge `0` nghĩa là “không có danh tính”, thường bị từ chối ở các giao diện
  cần phân quyền.
- Badge `0xFFFF` là superuser trong mô hình filesystem demo.
- Không được tự đổi badge của cap đã có; chỉ mint từ cap gốc có badge 0.

**Ví dụ:** badge giống số thẻ nhân viên in trên phong bì. Người nhận biết
phong bì đến từ phòng nào, nhưng không thể tự in lại số thẻ của người khác.

### 2.6. Untyped, retype, bump, watermark

Đây là cách ViFastOs quản lý bộ nhớ theo kiểu seL4:

- **Untyped**: một vùng nhớ vật lý thô, chưa có kiểu. Nó giống một mảnh đất
  chưa xây gì.
- **Retype**: biến một phần Untyped thành object có kiểu: Frame, PageTable,
  TCB, Endpoint, Notification, VSpace hoặc Untyped con.
- **Bump**: con trỏ cấp phát bên trong Untyped. Mỗi lần retype, bump tiến lên.
- **Watermark**: mốc/giới hạn đã dùng. Trong một số tài liệu cũ dùng để nói
  “đã cấp tới đâu”.
- **Accounting**: kernel ghi lại đã dùng bao nhiêu byte/bao nhiêu object, để
  không cấp quá khả năng.
- **Reserved**: số object mà Untyped cha giữ chỗ cho các Untyped con, tránh
  chuyện con bị “đói” vì anh em dùng hết quota.

**Ví dụ:** Untyped là khu đất. Retype là xây nhà kho, nhà ở, văn phòng trên
đất đó. Kernel không có “kho vật liệu chung” phát cho mọi người; ai giữ đất thì
người đó xây. Hết đất thì người đó nhận `OutOfMemory`, không ảnh hưởng người
khác.

### 2.7. Object table, ObjRef và generation

- **Object table**: bảng cố định chứa mọi object. Bản hiện tại có giới hạn
  khoảng 128–136 object, 16 TCB, 64 cap mỗi task.
- **ObjRef**: tham chiếu object gồm `{index, generation}`.
- **Generation**: số phiên bản. Khi một slot được tái sử dụng cho object mới,
  generation tăng lên. Mọi `ObjRef` cũ trỏ tới generation cũ sẽ bị từ chối.

**Ví dụ:** bạn thuê căn hộ số 5. Sau khi bạn trả nhà, chủ nhà cho người khác
thuê. Nếu bạn còn giữ chìa khóa cũ, ổ khóa đã đổi (generation mới), chìa cũ
không mở được. Đây là cách chống **use-after-free** mà không cần con trỏ thô.

### 2.8. Root task, supervisor, pager

- **Root task**: task user-space đầu tiên do kernel tạo. Nó nhận một ít
  capability và tự tạo các task/server khác. Đừng nhầm với “tài khoản root”
  của Linux. Trong ViFastOs, root task là “người kiến trúc sư” của userland.
- **Supervisor**: dịch vụ user-space nhận fault, quyết định restart task, giới
  hạn số lần lỗi, chuyển sang trạng thái degraded. Kernel chỉ gửi fault như
  một message; chính sách nằm ở supervisor.
- **Pager**: dịch vụ xử lý page fault. Khi một task truy cập trang chưa có,
  kernel báo cho pager; pager quyết định cấp trang nào, map vào đâu.
- **Name server**: dịch vụ tra cứu tên → endpoint/capability, giống “danh bạ”
  của hệ thống.
- **Compositor**: dịch vụ ghép cửa sổ/đồ họa, quyết định cửa sổ nào hiển thị
  trên cùng, nhận sự kiện chuột/bàn phím.

**Ví dụ:** root task giống ban quản lý tòa nhà khai trương các quầy dịch vụ.
Supervisor là bảo hiểm: khi một quầy cháy, quyết định mở lại hay đóng. Pager là
kho vật tư: khi một căn hộ cần thêm đồ, kho cấp.

### 2.9. Vì sao mô hình capability khó?

- Không có “quyền root” toàn năng; mọi thứ phải được cấp tường minh.
- Phải suy nghĩ về **ai giữ cap**, **ai được copy**, **thu hồi thế nào**.
- Khó tương thích với phần mềm kiểu POSIX/Linux vốn quen với uid/gid và
  “root có tất cả”.
- IPC và cap transfer làm tăng độ phức tạp so với gọi hàm thông thường.

Đổi lại, hệ thống có **least privilege** (quyền tối thiểu) và **thu hồi được**
quyền đã cấp — hai tính chất rất giá trị cho hệ thống an toàn.

**POSIX** (Portable Operating System Interface) là chuẩn API kiểu Unix
(`open`, `read`, `fork`, `pthread`…). ViFastOs **không** phải hệ POSIX; muốn
chạy app Linux cần một lớp tương thích hoặc máy ảo Linux.

### 2.10. KResult và các mã lỗi

Kernel không panic khi user làm sai. Mỗi syscall trả `KResult<T>` — hoặc
`Ok(value)`, hoặc `Err(KError)`. Các mã lỗi hay gặp:

| Mã lỗi | Nghĩa | Ví dụ |
|---|---|---|
| `InvalidCap` | Cap không tồn tại/sai slot | Dùng slot 0 hoặc slot trống. |
| `WrongType` | Cap trỏ sai loại object | Gọi `Recv` trên cap Frame. |
| `NoRights` | Thiếu quyền | Ghi file `O_RDONLY`, map W^X. |
| `SlotOccupied` | Slot đích đã có cap | Chuyển cap vào slot đã dùng. |
| `OutOfMemory` | Hết object/arena/queue | Retype khi Untyped đã hết. |
| `InvalidArg` | Tham số sai | Con trỏ user không hợp lệ, VA lệch trang. |
| `Alignment` | Địa chỉ không căn trang | Map frame ở địa chỉ lẻ. |
| `AlreadyMapped` | VA đã có mapping | Map đè. |
| `NotMapped` | VA chưa có mapping | Unmap trang chưa map. |
| `NoCaller` | `Reply` khi không có client | Server reply hai lần. |
| `ObjectDestroyed` | `ObjRef` cũ/object đã mất | Dùng cap sau revoke. |
| `MissingPageTable` | Thiếu bảng trang trung gian | `FrameMap` khi chưa có PageTable. |
| `IllegalOperation` | Trạng thái/bất biến sai | Xóa cap còn object con. |
| `NotImplemented` | Syscall chưa hỗ trợ | `SchedContextConfigure` hiện là placeholder. |
| `WouldBlock` | Server đang giữ caller khác | `Recv` khi chưa `Reply`. |
| `NotFound` | Không thấy file | Mở `/khong-ton-tai`. |

Nguyên tắc: **lỗi của user không bao giờ làm kernel panic**. `panic!` trong
kernel chỉ dành cho lỗi lập trình nội bộ (bất biến bị phá).

---

## 3. Giao tiếp: IPC, endpoint, notification

### 3.1. IPC là gì?

**IPC** (Inter-Process Communication) là cách các tiến trình trao đổi dữ liệu.
Trong monolithic kernel, hai thành phần trong kernel có thể gọi hàm trực tiếp.
Trong microkernel, driver/filesystem/app là các process riêng, nên phải gửi
message qua kernel. Đây là cái giá lớn nhất của microkernel.

### 3.2. Endpoint

**Endpoint** là một object “điểm hẹn” IPC đồng bộ. Nó có hai hàng đợi:

- **send queue**: các thread đang chờ gửi (`BlockedSend`);
- **recv queue**: các thread đang chờ nhận (`BlockedRecv`).

Hàng đợi trong bản hiện tại có giới hạn 8 phần tử, FIFO.

### 3.3. Các kiểu gửi/nhận

| Syscall | Ý nghĩa | Có block? |
|---|---|---|
| `Send` | Gửi message; nếu chưa có người nhận thì chờ | Có thể |
| `Recv` | Nhận message; nếu chưa có thì chờ | Có thể |
| `Call` | Gửi rồi chờ server trả lời (RPC) | Luôn chờ đến khi `Reply` |
| `Reply` | Server trả lời caller hiện tại | Không |
| `ReplyRecv` | Trả lời caller cũ rồi nhận request mới | Có thể |
| `Signal` | Bật bit trong notification | Không |
| `Wait` | Chờ notification, trả word | Có thể |
| `Poll` | Lấy word notification và clear, không chờ | Không |

**RPC** (Remote Procedure Call) ở đây không phải qua mạng, mà là “gọi hàm ở
process khác” thông qua IPC. `Call`/`Reply` là dạng RPC của ViFastOs.

### 3.4. Message, label, word, IPC buffer

- **Message**: cấu trúc dữ liệu gửi qua IPC. Bản hiện tại có `label`, `len`,
  `cap` (slot capability cần chuyển) và tối đa 8 word dữ liệu.
- **Label**: nhãn số để server biết đây là request gì (ví dụ `OPEN`, `READ`,
  `WRITE`).
- **Word**: đơn vị 64-bit.
- **IPC buffer**: vùng nhớ được map vào cả client và server để chuyển dữ liệu
  lớn hơn 8 word mà không phải copy qua lại quá nhiều lần. Trong ViFastOs,
  buffer 32 word có thể copy trực tiếp qua địa chỉ vật lý.
- **Shared memory**: vùng nhớ chung giữa hai process. Đây là cách nhanh nhất
  để chia sẻ dữ liệu lớn, nhưng phải quản lý quyền và đồng bộ cẩn thận.

**Ví dụ:** message là phong bì nhỏ. Label là loại đơn (“xin mở file”, “xin đọc
file”). IPC buffer là một bàn chung để hai bên đặt hàng hóa lớn, thay vì nhét
tất cả vào phong bì.

### 3.5. Notification — báo hiệu bất đồng bộ

**Notification** là object chứa một word bit. Nó giống “đèn báo”:

- `Signal` bật bit (OR vào word), đánh thức người đang chờ.
- `Wait` chờ đến khi có word; nếu chưa có thì block.
- `Poll` lấy word và clear, không block.
- Nếu chưa có ai chờ, word được tích lũy; lần `Wait` sau sẽ thấy.

**Bound notification**: notification gắn với TCB, để `Recv` trên endpoint có
thể trả về khi có tín hiệu. Đây là cơ chế để driver vừa chờ IPC vừa chờ ngắt.

**Ví dụ:** endpoint là quầy giao dịch; notification là đèn “có thư mới” ở cửa.
Bạn có thể đứng chờ ở quầy (blocking) hoặc nhìn đèn báo rồi mới quay lại.

### 3.6. Trạng thái block

| Trạng thái | Ý nghĩa |
|---|---|
| `Inactive` | Chưa cấu hình/không chạy. |
| `Ready` | Sẵn sàng chạy, đang nằm trong runqueue. |
| `Running` | Đang chạy trên CPU. |
| `BlockedSend` | Đang chờ gửi IPC. |
| `BlockedRecv` | Đang chờ nhận IPC. |
| `BlockedReply` | Đang chờ server trả lời (`Call`). |
| `BlockedNotification` | Đang chờ notification. |
| `Faulted` | Bị lỗi, đang chờ xử lý (fault endpoint/supervisor). |

### 3.7. Fault endpoint

Khi thread user bị lỗi (page fault, illegal instruction…), kernel không panic.
Nó gửi một message `LABEL_FAULT` tới **fault endpoint** đã cấu hình, kèm địa
chỉ lỗi và nguyên nhân. Thread chuyển sang `BlockedFault`; một handler có thể
`Recv`, quyết định restart hoặc bỏ qua. Đây là nền tảng cho **supervisor** và
**pager** sau này.

**Ví dụ:** nhân viên gặp sự cố không làm sập cả công ty; họ bấm chuông gọi
quản lý. Quản lý quyết định cho làm lại, chuyển việc khác hay dừng.

### 3.8. Các nhóm syscall chính

Không cần thuộc lòng số syscall, nhưng nên biết nhóm:

| Nhóm | Syscall tiêu biểu | Dùng để |
|---|---|---|
| IPC | `Send`, `Call`, `Recv`, `Reply`, `ReplyRecv` | Gửi/nhận message, RPC. |
| Notification | `Signal`, `Wait`, `Poll` | Báo hiệu bất đồng bộ. |
| Capability | `Retype`, `CapCopy`, `CapMint`, `CapDelete`, `CapRevoke` | Tạo object, copy/mint/thu hồi quyền. |
| Thread | `TcbConfigure`, `TcbResume`, `TcbSuspend`, `TcbSetPriority`, `TcbSetAffinity`, `TcbSetIpcBuffer` | Cấu hình/chạy/dừng thread. |
| Bộ nhớ | `PageTableMap`, `FrameMap`, `FrameUnmap` | Dựng page table, map/unmap frame. |
| IOMMU | `IoMap`, `IoUnmap` | Cấp/thu hồi vùng DMA cho thiết bị. |
| IRQ | `IrqGet`, `IrqSetNotification`, `IrqAck` | **placeholder**, chưa chạy đầy đủ. |
| Debug/thời gian | `DebugWrite`, `GetTime`, `Exit` | In log, đọc tick, thoát QEMU. |
| File | `Open`, `Read`, `Write`, `Close`, `Stat`, `Seek` | File descriptor. |
| Block | `BlockRead`, `BlockWrite`, `BlockFlush` | Đọc/ghi/flush sector; cần cap `BlockDevice`. |

**BootInfo** là trang dữ liệu read-only mà runtime map cho root task, chứa
magic `VFBOOT01`, version, các slot cap quan trọng và khoảng slot trống. Root
đọc nó để biết “những gì được trao” thay vì hard-code số slot.

**UserAction** là enum trong runtime mô tả kernel muốn runtime làm gì tiếp:
trả kết quả, copy user, map PTE, block để chuyển context, thoát… Nó là “phiếu
chuyển việc” giữa kernel portable và platform.

---

## 4. Thời gian: scheduler, SMP và thời gian thực

### 4.1. Thread, TCB và runqueue

- **TCB** (Thread Control Block) lưu mọi thứ kernel cần để chạy một thread.
- **Runqueue**: hàng đợi các thread `Ready`. Trong bản hiện tại, mỗi hart có
  một ready queue riêng; khi queue local rỗng, hart có thể **work-steal** thread
  từ hart khác (nếu affinity cho phép).
- **Affinity**: tập hart mà một thread được phép chạy. Ví dụ affinity `0b10`
  nghĩa là chỉ chạy trên hart 1.

### 4.2. Priority, round-robin, preemption

- **Priority**: số càng lớn càng được chạy trước.
- **Round-robin**: các thread cùng priority chạy lần lượt theo thứ tự FIFO.
- **Timeslice/quantum**: khoảng thời gian tối đa một thread được chạy trước
  khi bị xét nhường CPU.
- **Tick**: mỗi lần timer ngắt. Timer 100 Hz nghĩa là 100 tick/giây.
- **Preemption**: kernel dừng thread đang chạy để chạy thread khác. Không có
  preemption, một thread vòng lặp vô hạn có thể chiếm CPU mãi.
- **Idle**: khi không có thread nào, CPU chạy vòng chờ. Trên phần cứng, dùng
  lệnh `WFI` (Wait For Interrupt) để tiết kiệm điện.

### 4.3. SMP, hart, BSP, AP

- **SMP** (Symmetric Multiprocessing): nhiều CPU/core cùng chạy, chia sẻ bộ
  nhớ, cùng một kernel. Khác với **AMP** (Asymmetric MP), nơi mỗi core chạy
  một hệ thống khác nhau.
- **Hart** (Hardware Thread): thuật ngữ RISC-V chỉ một luồng phần cứng/CPU core
  mà kernel có thể lập lịch.
- **BSP** (Boot Strap Processor): trên x86, là CPU đầu tiên khởi động và chạy
  firmware. **BSP** còn có nghĩa **Board Support Package**: gói code hỗ trợ một
  board cụ thể (trong ViFastOs là các crate `vifast-platform-*`). Hai nghĩa này
  xuất hiện trong cùng dự án, phải nhìn ngữ cảnh.
- **AP** (Application Processor): các CPU phụ trên x86, được BSP đánh thức.
- **IPI** (Inter-Processor Interrupt): ngắt gửi từ hart này sang hart khác, dùng
  để đánh thức scheduler, yêu cầu TLB shootdown hoặc dừng khẩn cấp.
- **WFI** (Wait For Interrupt): lệnh cho CPU ngủ đến khi có ngắt.

### 4.4. BKL, spinlock, ticket lock

- **BKL** (Big Kernel Lock): một khóa lớn bảo vệ gần như toàn bộ kernel. Cách
  này đơn giản, dễ đúng, nhưng làm các syscall bị tuần tự hóa giữa các hart.
- **Spinlock**: khóa mà thread “quay vòng” chờ đến khi lấy được. Phù hợp cho
  vùng critical section rất ngắn trong kernel.
- **Ticket lock**: spinlock công bằng theo kiểu “lấy số thứ tự”; ai lấy số
  trước được vào trước.
- **RAII** (Resource Acquisition Is Initialization): kỹ thuật Rust/C++ tự động
  nhả khóa khi biến ra khỏi scope. Giúp tránh quên `unlock`.
- **Critical section**: đoạn code chỉ một thread/hart được vào tại một thời điểm.

**Ví dụ:** BKL là một cái cổng duy nhất vào tòa nhà. Ai cũng phải xếp hàng.
Muốn nhanh hơn phải mở nhiều cửa (khóa mịn), nhưng phải đảm bảo các cửa không
đâm vào nhau.

### 4.5. Thời gian thực: MCS, WCET, MCP

- **Real-time** (thời gian thực): hệ thống phải trả lời trong thời hạn xác định,
  không chỉ “nhanh trung bình”.
- **WCET** (Worst-Case Execution Time): thời gian xấu nhất của một đoạn code.
  Hệ thống an toàn cần biết WCET để đảm bảo không trễ deadline.
- **MCS** (Mixed Criticality Scheduling): lập lịch cho các task có mức quan
  trọng khác nhau, thường dùng **budget/period**.
- **SchedContext**: object chứa ngân sách CPU (budget) và chu kỳ (period).
- **Budget**: số thời gian CPU tối đa một task được dùng trong một period.
- **Period**: chu kỳ lặp lại của ngân sách.
- **MCP** (Maximum Controlled Priority): mức ưu tiên cao nhất mà một thread
  được phép đặt cho thread khác. Chống việc tự nâng priority.
- **Priority donation**: tạm nâng priority của thread giữ khóa để tránh
  priority inversion. ViFastOs chưa triển khai đầy đủ.
- **Priority inversion**: thread ưu tiên thấp giữ khóa mà thread ưu tiên cao
  đang cần, khiến thread cao bị chặn bởi thread thấp.

Trong ViFastOs, MCS/SchedContext mới là khái niệm/ABI đặt chỗ, chưa phải
triển khai hoàn chỉnh. Đừng thấy tên trong enum mà tưởng đã chạy.

### 4.6. SMP trong ViFastOs hiện tại

- Cả ba kiến trúc đã có 2 hart chạy scheduler/worker; x86_64 đã bật AP bằng
  ACPI MADT + LAPIC + chuỗi INIT-SIPI-SIPI.
- Có per-hart runqueue, work stealing, timer per-hart, IPI reschedule.
- Có TLB shootdown theo VSpace/VA; AArch64 dùng broadcast, RISC-V/x86 dùng
  targeted IPI.
- Vẫn còn **BKL**; khóa mịn và x2APIC là việc tiếp theo.
- Ý nghĩa thực tế: “SMP OK” trong log chứng minh nhiều hart đã online và phối
  hợp được, không có nghĩa mọi thứ đã chạy song song hoàn hảo.

---

## 5. Bộ nhớ: MMU, page table, TLB, W^X

### 5.1. VA, PA, page, frame

- **VA** (Virtual Address): địa chỉ ảo mà chương trình nhìn thấy.
- **PA** (Physical Address): địa chỉ thật trong RAM.
- **Page**: đơn vị bộ nhớ ảo, thường 4 KiB.
- **Frame**: đơn vị bộ nhớ vật lý cùng kích thước page, thường 4 KiB.
- **Page size** của dự án hiện tại là 4096 byte (4 KiB).

**Ví dụ:** VA giống số căn hộ trên giấy tờ; PA giống tọa độ thật của căn hộ
trong thành phố. MMU là người phiên dịch.

### 5.2. MMU, page table, PTE

- **MMU** (Memory Management Unit): phần cứng dịch VA → PA và kiểm tra quyền.
- **Page table**: bảng nhiều cấp chứa ánh xạ VA → PA.
- **PTE** (Page Table Entry): một mục trong bảng, chứa địa chỉ frame + quyền
  (read/write/execute/user).
- **Page walk**: quá trình phần cứng đi qua các cấp bảng để tìm PTE.
- **Leaf**: PTE ở cấp cuối, trỏ trực tiếp tới frame dữ liệu.
- **Table descriptor**: PTE trỏ tới bảng cấp tiếp theo, không phải dữ liệu.

### 5.3. Ba kiến trúc, ba kiểu page table

| Kiến trúc | Tên | Số cấp | Root register |
|---|---|---:|---|
| RISC-V 64 | **Sv39** (39-bit virtual address) | 3 | `satp` |
| AArch64 | 4-level, 4 KiB granule | 4 | `TTBR0` (user), `TTBR1` (kernel) |
| x86_64 | 4-level paging | 4 | `CR3` |

Từ viết tắt trên x86:

- **PML4** (Page Map Level 4): bảng gốc.
- **PDPT** (Page Directory Pointer Table): bảng cấp 2.
- **PD** (Page Directory): bảng cấp 3.
- **PT** (Page Table): bảng cấp 4, chứa PTE lá.
- **PML4/PDPT/PD/PT** chỉ là tên các cấp; đừng nhầm với “PT” = page table nói
  chung.

Trên AArch64:

- **TTBR0** giữ bảng trang cho user space.
- **TTBR1** giữ bảng trang cho kernel (high-half). ViFastOs chưa dùng TTBR1
  theo kiểu high-half đầy đủ.
- **TCR** (Translation Control Register), **MAIR** (Memory Attribute
  Indirection Register), **SCTLR** (System Control Register) là các thanh ghi
  cấu hình MMU.

### 5.4. VSpace

**VSpace** (Virtual Address Space) là không gian địa chỉ ảo của một tiến trình.
Nó gồm:

- root page table (handle tới bảng gốc);
- range VA được phép dùng;
- `next_va` để cấp VA tuần tự;
- **reverse map** các mapping để phục vụ unmap/revoke.

Trong ViFastOs, mỗi task/server/client có VSpace riêng. Đây là cách ly bộ nhớ
cơ bản: task A không thấy VA của task B nếu không được map.

**Lưu ý:** page table thật của phần cứng là nguồn sự thật; reverse map chỉ là
sổ ghi chú để kernel biết đã map những gì.

### 5.5. TLB, ASID, PCID, shootdown

- **TLB** (Translation Lookaside Buffer): cache ánh xạ VA → PA trong CPU. Nhờ
  TLB, page walk không phải làm lại mỗi lần truy cập.
- **ASID** (Address Space ID): nhãn gắn vào TLB entry để phân biệt các VSpace.
  Nhờ ASID, khi đổi process không cần xóa sạch TLB.
- **PCID** (Process Context ID): tương đương ASID trên x86.
- **TLB shootdown**: sau khi sửa page table/unmap, phải làm cho các hart khác
  xóa TLB entry cũ. Nếu không, hart khác vẫn đọc được dữ liệu cũ qua TLB.
- **IPI** thường được dùng để gửi yêu cầu shootdown.

**Ví dụ:** TLB là cuốn sổ tay ghi “địa chỉ A nằm ở đâu”. Khi bạn chuyển nhà
(sửa page table), bạn phải gọi tất cả những người đang giữ sổ tay cũ xóa trang
đó. Nếu không, họ vẫn đến địa chỉ cũ.

### 5.6. Map, unmap, page fault, pager

- **Map**: gắn một frame vật lý vào một VA trong VSpace, kèm quyền.
- **Unmap**: gỡ ánh xạ, thường phải flush TLB.
- **Page fault**: CPU báo lỗi khi truy cập VA chưa map hoặc không đủ quyền.
- **Pager**: dịch vụ quyết định cấp trang khi có page fault.
- **Demand paging**: chỉ cấp trang khi thực sự cần, thay vì nạp hết lúc đầu.
  ViFastOs mới có khái niệm pager/fault endpoint, chưa có demand paging đầy đủ.
- **Lazy allocation**: cấp phát “hứa” trước, chỉ map thật khi dùng.

### 5.7. W^X, NX, guard page, identity map

- **W^X** (Write xor Execute): một trang không được vừa ghi vừa thực thi. Nếu
  trang ghi được thì không chạy được, và ngược lại. Đây là lớp bảo vệ chống
  chèn mã độc vào vùng dữ liệu.
- **NX** (No-eXecute): bit cấm thực thi trên x86.
- **UXN/PXN** (User/Privileged eXecute Never): bit tương đương trên AArch64.
- **RO/RW/RX**: read-only, read-write, read-execute.
- **Guard page**: trang cố tình không map, đặt cạnh stack để phát hiện tràn
  stack (stack overflow) thay vì ghi đè dữ liệu khác.
- **Identity map**: ánh xạ VA = PA. Kernel thường dùng identity map cho vùng
  RAM/MMIO của chính nó để dễ truy cập.
- **High-half kernel**: kernel đặt ở nửa trên không gian địa chỉ ảo, user ở
  nửa dưới. ViFastOs chưa làm đầy đủ.

Trong ViFastOs, W^X được kiểm tra **theo Frame**, không chỉ theo từng mapping:
nếu frame đã có một leaf ghi được, không được map nó thành executable ở VA khác.
Nhờ vậy hai alias của cùng một frame không lách được W^X.

### 5.8. MMIO, DMA, IOMMU

- **MMIO** (Memory-Mapped I/O): dùng địa chỉ bộ nhớ để đọc/ghi thanh ghi thiết
  bị. Ví dụ UART ở địa chỉ `0x1000_0000`.
- **DMA** (Direct Memory Access): thiết bị tự đọc/ghi RAM mà không qua CPU.
  Nhanh, nhưng nguy hiểm nếu thiết bị ghi vào vùng kernel.
- **IOMMU** (I/O Memory Management Unit): MMU cho thiết bị. Nó dịch địa chỉ
  thiết bị dùng (IOVA) sang PA và giới hạn vùng DMA.
- **IOVA**: địa chỉ mà thiết bị nhìn thấy.
- **IoSpace**: bảng dịch IOMMU của một thiết bị/domain.
- **IOTLB**: TLB của IOMMU.
- **VT-d**: công nghệ IOMMU của Intel.
- **SMMU** (System Memory Management Unit): IOMMU của ARM.
- **DMAR** (DMA Remapping Reporting): bảng ACPI mô tả IOMMU của Intel.
- **IORT** (I/O Remapping Table): bảng ACPI mô tả SMMU trên ARM.

ViFastOs hiện có **IOMMU phần mềm** (domain + reverse map + `IoMap`/`IoUnmap`),
chưa có IOMMU phần cứng. Nghĩa là mô hình quyền đã có, nhưng cách ly DMA thật
trên phần cứng vẫn là việc tiếp theo.

### 5.9. Cache, I-cache, fence

- **Cache**: bộ nhớ đệm nhỏ, nhanh, giữ dữ liệu hay dùng.
- **I-cache** (Instruction cache): cache mã lệnh.
- **D-cache** (Data cache): cache dữ liệu.
- **Cache coherence**: các core nhìn thấy cùng một giá trị bộ nhớ.
- **`fence.i`** (RISC-V): đồng bộ I-cache với dữ liệu vừa ghi.
- **`sfence.vma`** (RISC-V): flush TLB theo địa chỉ/ASID.
- **`tlbi`** (AArch64): TLB invalidate.
- **`invlpg`** (x86): invalidate page trong TLB.
- **`dsb`/`isb`** (AArch64): barrier đồng bộ bộ nhớ/instruction stream.

Khi map một trang thành executable sau khi ghi code vào đó, kernel phải đồng bộ
I-cache; nếu không, CPU có thể chạy mã cũ. ViFastOs gọi `sync_icache` khi map
`EXEC`.

### 5.10. PageTableOps, reverse map và PA uniqueness

- **`PageTableOps`**: trait mà platform cung cấp cho kernel portable để đọc/ghi
  PTE, zero frame, flush TLB, đồng bộ I-cache. Nhờ nó, `vifast-kernel` không
  cần biết Sv39 hay PML4.
- **Reverse map**: `VSpace.mappings` ghi lại những mapping đã tạo, phục vụ
  unmap/revoke. Page table thật vẫn là nguồn sự thật của phần cứng.
- **PA uniqueness**: bất biến cấm hai object vật lý (Frame/PageTable/VSpace
  root+aux) chồng dải địa chỉ. Nếu hai Frame cùng PA, bộ đếm W/X sẽ sai và
  W^X có thể bị lách.
- **Frame.mapped_w/mapped_x**: đếm số leaf đang WRITE/EXEC trên **mọi** VSpace.
  Đây là lý do hai alias của cùng một frame không thể tạo ra một trang vừa ghi
  vừa chạy.

### 5.11. Những gì bộ nhớ ViFastOs chưa có

- Page-table page allocator tổng quát.
- Thu hồi frame/object ngoài đường revoke Untyped.
- Demand paging đầy đủ, swap, copy-on-write.
- Superpage/large page (2 MiB/1 GiB).
- High-half kernel đầy đủ.
- IOMMU phần cứng.
- TLB shootdown cho mọi trường hợp (đã có per-VA nhưng vẫn đang hoàn thiện).

---

## 6. Khởi động và phần cứng

### 6.1. Boot chain của ba kiến trúc

**RISC-V:**

```text
QEMU
  → OpenSBI (firmware ở M-mode)
  → kernel ViFastOs (S-mode)
  → user mode (U-mode) qua sret
```

**AArch64:**

```text
QEMU virt
  → kernel ELF (có thể bắt đầu ở EL2, hạ xuống EL1)
  → user mode (EL0) qua eret
```

**x86_64:**

```text
OVMF (UEFI firmware)
  → FAT EFI System Partition
  → EFI/BOOT/BOOTX64.EFI (kernel dạng PE/COFF)
  → ring 3 qua iretq
```

### 6.2. Từ vựng RISC-V

| Từ | Nghĩa | Ghi chú |
|---|---|---|
| **RISC-V** | Reduced Instruction Set Computer V | ISA mở, gọn, phù hợp nghiên cứu. |
| **CSR** | Control and Status Register | Thanh ghi cấu hình/trạng thái của RISC-V (ví dụ `satp`, `stvec`, `scause`). |
| **SBI** | Supervisor Binary Interface | Giao diện giữa kernel S-mode và firmware M-mode. |
| **OpenSBI** | Open Supervisor Binary Interface | Firmware mã nguồn mở cung cấp SBI, thường dùng với QEMU. |
| **HSM** | Hart State Management | Phần SBI để start/stop hart. |
| **SRST** | System Reset | Phần SBI để reset/tắt máy. |
| **PLIC** | Platform-Level Interrupt Controller | Bộ điều khiển ngắt ngoài của RISC-V. |
| **CLINT** | Core-Local Interruptor | Bộ timer/IPI cục bộ của RISC-V. |
| **PMP** | Physical Memory Protection | Cơ chế bảo vệ vùng nhớ vật lý của RISC-V. |
| **satp** | Supervisor Address Translation and Protection | Thanh ghi giữ root page table + ASID. |
| **stvec** | Supervisor Trap Vector | Địa chỉ bảng xử lý trap. |
| **sepc** | Supervisor Exception Program Counter | PC tại thời điểm trap. |
| **scause** | Supervisor Cause | Mã nguyên nhân trap. |
| **sscratch** | Supervisor Scratch | Thanh ghi tạm, dùng để giữ con trỏ trap stack. |
| **SUM** | Supervisor User Memory | Bit cho phép kernel truy cập bộ nhớ user khi copy. |
| **sret** | Supervisor Return | Lệnh quay về U-mode. |
| **ecall** | Environment Call | Lệnh syscall của RISC-V. |

### 6.3. Từ vựng ARM/AArch64

| Từ | Nghĩa | Ghi chú |
|---|---|---|
| **AArch64** | ARM Architecture 64-bit | Kiến trúc ARM 64-bit. |
| **EL0/EL1/EL2/EL3** | Exception Levels | Cấp đặc quyền; EL0 user, EL1 kernel, EL2 hypervisor, EL3 secure. |
| **PSCI** | Power State Coordination Interface | Giao diện firmware để bật/tắt CPU, reboot. |
| **GIC** | Generic Interrupt Controller | Bộ điều khiển ngắt của ARM. |
| **GICv2** | GIC version 2 | Phiên bản dự án dùng trên QEMU `virt`. |
| **PPI** | Private Peripheral Interrupt | Ngắt riêng của từng CPU (ví dụ timer PPI 30). |
| **SPI** | Shared Peripheral Interrupt | Ngắt từ thiết bị dùng chung. |
| **SGI** | Software Generated Interrupt | Ngắt phần mềm, dùng cho IPI. |
| **TTBR0/TTBR1** | Translation Table Base Register | Thanh ghi root page table cho user/kernel. |
| **TCR** | Translation Control Register | Cấu hình MMU. |
| **MAIR** | Memory Attribute Indirection Register | Cấu hình thuộc tính bộ nhớ. |
| **SCTLR** | System Control Register | Bật/tắt MMU, cache… |
| **PAN** | Privileged Access Never | Cấm kernel truy cập user memory trừ khi tạm mở. |
| **UXN/PXN** | User/Privileged Execute Never | Cấm thực thi. |
| **VBAR** | Vector Base Address Register | Địa chỉ vector table. |
| **ESR** | Exception Syndrome Register | Mô tả nguyên nhân exception. |
| **FAR** | Fault Address Register | Địa chỉ gây fault. |
| **SP_EL0/SP_EL1** | Stack Pointer | Stack user/kernel. |
| **eret** | Exception Return | Quay về EL0. |
| **svc** | Supervisor Call | Lệnh syscall của ARM. |
| **WFI** | Wait For Interrupt | Ngủ chờ ngắt. |

### 6.4. Từ vựng x86_64

| Từ | Nghĩa | Ghi chú |
|---|---|---|
| **x86_64** | — | Kiến trúc Intel/AMD 64-bit. |
| **ring 0/3** | Privilege rings | Kernel/user. |
| **GDT** | Global Descriptor Table | Bảng mô tả segment. |
| **TSS** | Task State Segment | Lưu stack kernel cho ring0 (`RSP0`), IST… |
| **IDT** | Interrupt Descriptor Table | Bảng handler ngắt/exception. |
| **IST** | Interrupt Stack Table | Stack riêng cho một số exception nguy hiểm. |
| **RSP0** | Ring 0 Stack Pointer | Stack kernel khi vào từ user. |
| **CR3** | Control Register 3 | Giữ địa chỉ PML4. |
| **PML4/PDPT/PD/PT** | Page table levels | Bốn cấp page table. |
| **NXE** | No-eXecute Enable | Bit bật NX trong `EFER`. |
| **SMEP** | Supervisor Mode Execution Prevention | Cấm kernel thực thi code user. |
| **SMAP** | Supervisor Mode Access Prevention | Cấm kernel truy cập dữ liệu user nếu không mở. |
| **STAC/CLAC** | Set/Clear AC flag | Tạm mở/đóng SMAP khi copy user. |
| **LAPIC** | Local Advanced Programmable Interrupt Controller | Bộ điều khiển ngắt cục bộ của từng CPU. |
| **x2APIC** | Extended x2APIC | Phiên bản LAPIC mở rộng. |
| **IRR** | Interrupt Request Register | Thanh ghi các ngắt đang chờ. |
| **PIC** | Programmable Interrupt Controller | Chip 8259 legacy; cũng có thể là Position-Independent Code. |
| **PIT** | Programmable Interval Timer | Timer 8254 legacy. |
| **TSC** | Time Stamp Counter | Bộ đếm chu kỳ CPU. |
| **SIPI** | Startup IPI | IPI đánh thức AP. |
| **INIT** | Initialization IPI | IPI đưa AP về trạng thái khởi tạo. |
| **ACPI** | Advanced Configuration and Power Interface | Chuẩn mô tả phần cứng/tiết kiệm điện. |
| **MADT** | Multiple APIC Description Table | Bảng ACPI liệt kê CPU/LAPIC. |
| **RSDP/XSDT** | Root/Extended System Description Table | Bảng gốc/ mở rộng của ACPI. |
| **UEFI** | Unified Extensible Firmware Interface | Firmware chuẩn thay BIOS. |
| **OVMF** | Open Virtual Machine Firmware | UEFI firmware cho QEMU. |
| **ESP** | EFI System Partition | Phân vùng FAT chứa file `.efi`; cũng có thể là Extended Stack Pointer. |
| **PE/COFF** | Portable Executable / Common Object File Format | Định dạng file `.efi` trên Windows/UEFI. |
| **GOP** | Graphics Output Protocol | Giao diện framebuffer của UEFI. |
| **ATA/PIO** | Advanced Technology Attachment / Programmed I/O | Chuẩn đĩa cũ; PIO là đọc/ghi qua CPU. |
| **AHCI** | Advanced Host Controller Interface | Giao diện SATA hiện đại. |
| **NVMe** | Non-Volatile Memory Express | Chuẩn SSD tốc độ cao. |
| **LBA** | Logical Block Addressing | Địa chỉ block logic trên đĩa. |

### 6.5. Firmware và “ai khởi động ai?”

- **Firmware** là phần mềm chạy trước kernel, nằm trong ROM/flash.
- **Bootloader** là chương trình nạp kernel.
- **OpenSBI** và **OVMF** là firmware/boot môi trường cho RISC-V/x86.
- **QEMU** là máy ảo/emulator dùng để chạy thử kernel mà không cần phần cứng
  thật.

### 6.6. Trampoline, real mode và port I/O

- **Trampoline**: đoạn code nhỏ được đặt ở địa chỉ thấp (< 1 MiB) để đánh thức
  AP trên x86. AP bắt đầu ở **real mode** (16-bit), nhảy qua trampoline để vào
  **protected mode** (32-bit) rồi **long mode** (64-bit).
- **Real mode / protected mode / long mode**: ba chế độ CPU của x86. Real mode
  là chế độ cũ 16-bit; long mode là 64-bit hiện đại.
- **Port I/O**: đọc/ghi thiết bị qua cổng I/O (`in`/`out`), khác MMIO. COM1
  trên x86 dùng port I/O; UART RISC-V/ARM dùng MMIO.
- **PCI/PCIe**: bus thiết bị phổ biến trên x86. VirtIO có thể chạy trên PCI
  (x86) hoặc MMIO (ARM/RISC-V).
- **PIO** trong “ATA PIO” là **Programmed I/O**: CPU tự đọc/ghi từng word với
  đĩa, không dùng DMA.

---

## 7. Lưu trữ: VFS, filesystem, ổ đĩa

### 7.1. VFS và FS khác nhau thế nào?

- **FS** (File System): cách tổ chức file trên một thiết bị: inode, thư mục,
  block, journal…
- **VFS** (Virtual File System): lớp giao diện chung để chương trình dùng
  `open/read/write/close` mà không cần biết bên dưới là FS gì.

Trong ViFastOs:

- kernel có một **ramfs** nhỏ để self-test/fallback;
- **VFS server** chạy ở user space, nói chuyện với client qua IPC;
- server dùng crate `vifast-fs` để đọc/ghi đĩa thật (VirtIO/ATA).

### 7.2. Block device, sector, LBA

- **Block device**: thiết bị đọc/ghi theo khối, ví dụ ổ đĩa.
- **Sector**: đơn vị nhỏ nhất, thường 512 byte.
- **Block**: đơn vị logic của filesystem, trong dự án là 512 byte.
- **LBA** (Logical Block Addressing): địa chỉ block trên đĩa, bắt đầu từ 0.
- **Disk image**: file giả lập ổ đĩa trong QEMU.

### 7.3. Superblock, inode, bitmap, directory entry

- **Superblock**: “chứng minh thư” của filesystem: magic, version, block size,
  số inode, vị trí bitmap, root inode…
- **Magic**: số/chuỗi nhận diện định dạng, ví dụ `VF04`.
- **Inode** (Index Node): metadata của một file/thư mục: loại, quyền, uid,
  kích thước, số link, danh sách block dữ liệu.
- **Bitmap**: bảng bit đánh dấu block/inode nào đã dùng.
- **Directory entry**: một dòng trong thư mục, ánh xạ tên → inode.
- **Root inode**: inode của thư mục gốc `/`.
- **UID** (User ID): số định danh người dùng.
- **Mode**: quyền file (owner/group/other, read/write/execute).
- **nlink**: số hard link trỏ tới inode.

### 7.4. Direct, single-indirect, double-indirect

Một inode không thể chứa địa chỉ của hàng nghìn block trong 64 byte. Vì vậy
filesystem dùng:

- **Direct block**: inode trỏ thẳng tới block dữ liệu.
- **Single-indirect**: inode trỏ tới một block chứa danh sách địa chỉ block.
- **Double-indirect**: trỏ tới block chứa danh sách block, mỗi block lại chứa
  danh sách địa chỉ block.

**Ví dụ:** direct là “nhà A, nhà B” ghi thẳng trong sổ. Single-indirect là
“mở trang 5 của sổ phụ”. Double-indirect là “mở trang 5 của sổ phụ, rồi mở
trang 3 của sổ phụ con”.

### 7.5. Journal, WAL, commit, replay, fsck

- **Journal**: nhật ký ghi trước khi thay đổi dữ liệu thật, để nếu mất điện
  giữa chừng có thể phục hồi.
- **WAL** (Write-Ahead Log): ghi log trước, áp dụng sau.
- **Transaction**: một nhóm thay đổi được coi là một đơn vị.
- **Commit**: ghi dấu “transaction này đã hoàn tất”.
- **CRC32**: checksum kiểm tra dữ liệu có bị hỏng không.
- **Replay**: đọc journal và áp dụng lại transaction đã commit sau khi crash.
- **Torn write**: ghi dở dang, chỉ một phần sector được ghi.
- **FSCK** (File System Check): kiểm tra và sửa lỗi filesystem.
- **Mount**: gắn filesystem vào cây thư mục để dùng.
- **Format**: tạo filesystem mới trên thiết bị.
- **Migrate**: nâng cấp định dạng cũ lên định dạng mới.

ViFastOs dùng journal write-ahead: ghi dữ liệu vào vùng journal, ghi header,
ghi commit sector có CRC32, sau đó mới ghi vào vị trí thật; cuối cùng xóa
commit. Khi mount, nếu thấy commit hợp lệ thì replay; nếu commit hỏng thì bỏ,
giữ dữ liệu cũ.

### 7.6. File descriptor và các thao tác

- **fd** (File Descriptor): số nhỏ đại diện cho file đang mở trong một process.
- **offset**: vị trí đọc/ghi hiện tại trong file.
- **seek**: đặt lại offset.
- **stat**: lấy metadata (kích thước, quyền…).
- **open flags**:
  - `O_RDONLY`: chỉ đọc;
  - `O_WRONLY`: chỉ ghi;
  - `O_RDWR`: đọc và ghi;
  - `O_CREATE`: tạo file nếu chưa có.
- **`/motd`**: “message of the day”, file demo chứa lời chào.
- **readdir**: đọc danh sách entry trong thư mục.
- **truncate**: cắt file ngắn lại (hoặc về 0).
- **unlink**: xóa tên file khỏi thư mục; dữ liệu chỉ mất khi nlink = 0.
- **rename**: đổi tên/di chuyển file.

### 7.7. Hard link và symlink

- **Hard link**: thêm một tên khác trỏ tới **cùng inode**. Hai tên bình đẳng;
  xóa một tên không mất dữ liệu nếu tên còn lại vẫn tồn tại. `nlink` đếm số
  tên.
- **Symlink** (symbolic link): file đặc biệt chứa **đường dẫn** tới file khác.
  Nó giống “lối tắt”; nếu file đích mất, symlink bị “gãy”.

**Ví dụ:** hard link giống hai cửa cùng mở vào một phòng. Symlink giống tấm
biển “phòng họp ở tầng 3”; nếu phòng đó bị dỡ, biển vẫn còn nhưng dẫn tới
hư không.

### 7.8. Cache, fsync, flush

- **Block cache**: giữ một số block trong RAM để đọc nhanh.
- **Write-through**: ghi xuống thiết bị ngay, rồi cập nhật cache. Không có
  dirty block, đơn giản và an toàn hơn.
- **Write-back**: chỉ ghi vào cache, lúc sau mới ghi xuống đĩa. Nhanh hơn nhưng
  mất điện có thể mất dữ liệu.
- **Dirty block**: block trong cache đã sửa nhưng chưa ghi xuống đĩa.
- **flush**: yêu cầu thiết bị ghi hết cache nội bộ của nó xuống media.
- **fsync**: đảm bảo dữ liệu của file đã thực sự xuống thiết bị, không chỉ nằm
  trong cache.

ViFastOs dùng write-through cache 8 slot; `fsync` gọi `BlockFlush` xuống
VirtIO (request type 4) hoặc ATA (lệnh `0xE7`).

### 7.9. VirtIO, ATA, polling

- **VirtIO**: chuẩn thiết bị ảo hóa, cho kernel thấy “ổ đĩa/ mạng/ bàn phím”
  như thiết bị thật.
- **VirtIO-MMIO**: phiên bản VirtIO dùng MMIO, không cần PCI.
- **Virtqueue**: hàng đợi mô tả request giữa driver và thiết bị.
- **Descriptor/Available/Used ring**: các vòng mô tả buffer.
- **Polling**: driver chủ động hỏi thiết bị “xong chưa?”, thay vì chờ ngắt.
  Đơn giản nhưng tốn CPU.
- **Interrupt-driven**: thiết bị báo ngắt khi xong. Hiệu quả hơn nhưng phức tạp
  hơn.
- **ATA PIO**: cách cũ đọc/ghi đĩa qua CPU, dùng cho QEMU x86.
- **AHCI/NVMe**: chuẩn đĩa hiện đại, chưa triển khai.

### 7.10. Giao thức VFS và cap block device

VFS server dùng các **label** (mã request) để phân biệt thao tác:

| Label | Request | Ý nghĩa |
|---|---|---|
| `OPEN` (`0x2001`) | Mở file | Trả fd hoặc lỗi. |
| `READ` (`0x2002`) | Đọc file | Dữ liệu trả qua IPC buffer. |
| `WRITE` (`0x2003`) | Ghi file | Dữ liệu lấy từ IPC buffer. |
| `CLOSE` (`0x2004`) | Đóng file | Giải phóng fd. |
| `STAT` (`0x2005`) | Lấy metadata | Kích thước, quyền… |
| `MKDIR`, `READDIR`, `UNLINK`, `RENAME`, `LINK`, `TRUNCATE`, `FSYNC`, `FSCK` | Mở rộng | Thư mục, hard link, đồng bộ, kiểm tra. |

Block device cũng là một **object/capability**. Muốn gọi `BlockRead` /
`BlockWrite` / `BlockFlush`, client phải có cap `BlockDevice` với quyền phù hợp.
Client không có cap sẽ bị từ chối — đây là ví dụ “không có vé thì không có
quyền”.

**ACL** (Access Control List) là danh sách quyền chi tiết cho từng người/nhóm.
Dự án mới có mode owner/other và UID, chưa có ACL đầy đủ.

### 7.11. Giới hạn filesystem hiện tại

- Ảnh 4 MiB, 64 inode, block 512 byte.
- Tên tối đa 28 byte, path tối đa 128 byte.
- File tối đa lý thuyết khoảng 8 MiB (VF04), nhưng bị giới hạn bởi ảnh 4 MiB.
- Ghi file lớn được chia thành transaction theo block; **mỗi block nguyên tử**,
  không phải cả file.
- Chưa có barrier phần cứng đầy đủ; giả định ghi một sector 512 byte là nguyên
  tử.
- Chưa có quota, ACL, journaling đa thiết bị, snapshot, encryption.

---

## 8. Rust, build và kiểm thử

### 8.1. Từ vựng Rust

| Từ | Nghĩa | Ghi chú |
|---|---|---|
| **Rust** | Ngôn ngữ lập trình | An toàn bộ nhớ, không cần garbage collector. |
| **Cargo** | Build system/package manager | Quản lý crate, dependency, test. |
| **Crate** | Đơn vị biên dịch/package | Ví dụ `vifast-kernel`. |
| **Workspace** | Tập nhiều crate | Repo này là một workspace. |
| **`no_std`** | Không dùng standard library | Kernel không có `std`, chỉ dùng `core`/`alloc` hạn chế. |
| **`alloc`** | Thư viện cấp phát | Cần global allocator; kernel cố gắng tránh. |
| **Heap** | Vùng nhớ cấp phát động | Kernel hiện dùng một số cấu trúc `alloc`/static; mục tiêu là bỏ dần. |
| **Static storage** | Vùng nhớ tĩnh | `Kernel` đặt trong static thay vì stack. |
| **`unsafe`** | Khối code vượt qua kiểm tra an toàn | Chỉ dùng ở platform/arch cho MMIO/asm; kernel/runtime `forbid(unsafe_code)`. |
| **`forbid(unsafe_code)`** | Cấm `unsafe` | Crate compiler sẽ từ chối nếu có `unsafe`. |
| **`repr(C)`** | Layout kiểu C | Bắt buộc cho struct đi qua biên kernel/user. |
| **Ownership/borrow** | Quyền sở hữu/mượn | Rust kiểm tra vòng đời, tránh use-after-free/double-free. |
| **Lifetime** | Vòng đời tham chiếu | Compiler đảm bảo tham chiếu không sống lâu hơn dữ liệu. |
| **Trait** | Giao diện/abstraction | Ví dụ `Arch`, `Platform`, `BlockDev`. |
| **Generic** | Lập trình tổng quát | Một code dùng cho nhiều kiểu. |
| **Macro** | Mã sinh mã | `println!`, `asm!`… |
| **`panic`** | Lỗi không phục hồi | Kernel dùng `panic = "abort"`; panic = lỗi lập trình. |
| **`Result<T, E>`** | Kiểu lỗi có thể xử lý | Kernel trả `KResult<T>` thay vì panic. |

**Vì sao Rust?** Kernel cần an toàn bộ nhớ. Rust giúp giảm lỗi con trỏ, tràn
buffer, use-after-free — những lỗi rất nguy hiểm trong kernel. Nhưng Rust không
tự làm kernel đúng; `unsafe` vẫn cần cho phần cứng.

### 8.2. Build, target, linker, ELF

- **Target triple**: tên nền tảng biên dịch, ví dụ
  `riscv64gc-unknown-none-elf`, `aarch64-unknown-none`,
  `x86_64-unknown-uefi`.
- **Cross-compile**: biên dịch trên máy này cho kiến trúc khác.
- **Linker script**: file mô tả code/data/stack đặt ở địa chỉ nào.
- **ELF** (Executable and Linkable Format): định dạng file thực thi trên
  Unix/RISC-V/ARM.
- **ELF64**: ELF 64-bit.
- **ET_EXEC**: file thực thi có địa chỉ cố định.
- **ET_DYN**: file có thể nạp ở địa chỉ động (PIE/shared library).
- **PT_LOAD**: segment cần nạp vào bộ nhớ.
- **PT_INTERP/PT_DYNAMIC/PT_TLS**: các segment cho dynamic linker/TLS; dự án
  từ chối để giữ user binary đơn giản.
- **BSS**: vùng dữ liệu chưa khởi tạo, được zero khi nạp.
- **Segment**: phần của file thực thi (code, data, stack…).
- **Symbol**: tên hàm/biến trong file.
- **Relocation**: điều chỉnh địa chỉ khi nạp.
- **PIE** (Position-Independent Executable): executable có thể nạp ở địa chỉ
  khác.
- **PIC** (Position-Independent Code): code không phụ thuộc địa chỉ tuyệt đối.
  (Cẩn thận: PIC còn là Programmable Interrupt Controller.)
- **Soft-float**: không dùng FPU/SIMD, tránh phải lưu context FP khi context
  switch.
- **FP/SIMD**: floating point / Single Instruction Multiple Data.
- **LTO** (Link-Time Optimization): tối ưu khi link.
- **`panic = "abort"`**: panic thì dừng, không unwind.
- **Page-aligned**: địa chỉ chia hết cho 4096.

### 8.3. Kiểm thử

| Từ | Nghĩa | Trong dự án |
|---|---|---|
| **Self-test** | Test chạy bên trong kernel lúc boot | Kiểm tra object/cap/IPC/scheduler/VSpace/ramfs. |
| **Host test** | Test chạy trên máy phát triển | `cargo test -p vifast-kernel`… |
| **QEMU test** | Test chạy kernel trong máy ảo | `scripts/test-qemu.py` chạy 3 kiến trúc. |
| **Harness** | Khung chạy test + kiểm tra log | Script Python đọc log QEMU, so khớp chuỗi. |
| **Assertion** | Điều kiện bắt buộc | Ví dụ log phải có `SELFTEST PASS`. |
| **Negative test** | Test đầu vào sai phải bị từ chối | Ví dụ map W+X phải trả lỗi. |
| **Regression test** | Test chống tái phát lỗi cũ | Chạy lại sau mỗi thay đổi. |
| **Smoke test** | Test nhanh, cơ bản | Kiểm tra “có chạy không”. |
| **Soak test** | Chạy lâu để tìm lỗi hiếm | Chạy lặp nhiều lần. |
| **Fuzz** | Sinh đầu vào ngẫu nhiên | Tìm crash/panic với dữ liệu xấu. |
| **Invariant** | Bất biến | Điều kiện luôn đúng; `check_invariants()` kiểm tra. |
| **CI** | Continuous Integration | Tự động build/test mỗi commit. |
| **Baseline** | Mốc chuẩn | Commit/test đang xanh để so sánh. |

**Vì sao cần nhiều tầng test?** Vì kernel chạy ở mức thấp: một lỗi nhỏ có thể
làm treo máy, mất dữ liệu, hoặc tạo lỗ hổng. Test host nhanh, QEMU gần phần
cứng hơn, fuzz tìm đầu vào lạ, invariant bắt trạng thái sai.

### 8.4. Kiểm chứng hình thức

- **Formal verification** (kiểm chứng hình thức): dùng toán học để chứng minh
  chương trình thỏa mãn đặc tả, không chỉ test vài trường hợp.
- **Model checking**: duyệt các trạng thái của mô hình để tìm vi phạm.
- **Kani**: công cụ model checking cho Rust.
- **Verus**: ngôn ngữ/công cụ chứng minh cho Rust.
- **loom**: thư viện test concurrency, thử nhiều thứ tự thực thi để tìm race.
- **Miri**: trình thông dịch Rust phát hiện undefined behavior trong code
  `unsafe`/concurrency ở mức host.
- **WCET** (đã giải thích): cần cho hệ thời gian thực.

ViFastOs **chưa** kiểm chứng hình thức; mới có invariant checker và fuzz hạn
chế. Đây là mục tiêu dài hạn, không phải tính năng đã có.

**TLS** (Thread Local Storage) là vùng dữ liệu riêng của từng thread. Parser
ELF của dự án từ chối `PT_TLS` để giữ user binary đơn giản; nếu sau này cần
thread-local, phải thiết kế thêm.

### 8.5. QEMU và các tùy chọn hay gặp

- **QEMU**: máy ảo/emulator.
- **`-machine virt`**: board ảo cho RISC-V/ARM.
- **`-smp 2`**: 2 CPU.
- **`-kernel`**: nạp kernel trực tiếp.
- **`-bios default`**: dùng OpenSBI mặc định.
- **`-nographic`**: không mở cửa sổ đồ họa, dùng serial.
- **OVMF**: firmware UEFI cho x86.
- **TCG** (Tiny Code Generator): bộ dịch động của QEMU; không phải tăng tốc
  phần cứng.
- **KVM**: tăng tốc ảo hóa trên Linux (không bắt buộc).

---

## 9. Dịch vụ, profile và lộ trình

### 9.1. Desktop và Space

ViFastOs có hai “profile” mục tiêu:

| | Desktop | Space |
|---|---|---|
| Mục đích | Máy tính cá nhân, máy trạm | Hệ thống an toàn sống còn (máy bay, vệ tinh…) |
| Tạo tiến trình lúc chạy | Có | Không (tĩnh sau boot) |
| Cắm thiết bị lúc chạy | Có | Không |
| MCS | Tùy chọn | Nên dùng cho task ít quan trọng |
| Cách khóa | Không khóa | Xóa untyped gốc, IRQ/sched control sau boot |

Điểm hay: **kernel không có “chế độ tĩnh” riêng**. Tính tĩnh đạt được bằng
cách không task nào còn giữ capability để tạo object mới. Cùng một kernel,
cùng bộ test, hai cấu hình.

### 9.2. Các dịch vụ dự kiến

- **Name server**: tra cứu tên → endpoint.
- **Supervisor**: nhận fault, restart task.
- **Pager**: cấp trang khi page fault.
- **Driver**: VirtIO console/block/net/input.
- **Filesystem server**: đã có bản user-space.
- **Network stack**: dự kiến dùng `smoltcp` hoặc tự viết.
- **Compositor**: ghép cửa sổ, quản lý input/focus.
- **ViFastKey**: bộ gõ tiếng Việt, chạy như dịch vụ user-space.
- **Payload**: ứng dụng/dịch vụ chạy trên hệ thống.

### 9.3. ViFastKey là gì?

**ViFastKey** là bộ gõ tiếng Việt của dự án. Nó không cần kernel làm gì đặc
biệt: nhận sự kiện bàn phím từ input service, xử lý thành chữ Việt, gửi text
tới ứng dụng/compositor qua IPC. Đây là ví dụ tốt cho triết lý “cơ chế ở
kernel, chính sách ở user space”.

### 9.4. “Modern” của dự án nằm ở đâu?

Không phải ở chỗ dùng công nghệ mới nhất, mà ở các ý tưởng:

- **Capability-based security** thay vì uid/root truyền thống.
- **Microkernel** với driver/FS ở user space.
- **Least privilege** và thu hồi quyền.
- **Multi-arch** với lõi portable.
- **SMP** và TLB shootdown.
- **Journaling filesystem** ở user space.
- **IOMMU** cho DMA (dù mới là mô hình).
- **Rust** và tách `unsafe`.
- **Test harness, fuzz, invariant**, hướng tới formal verification.
- **Hai profile** desktop/space.

---

## 10. Bảng tra nhanh A–Z

Bảng này để tra cứu nhanh; giải thích chi tiết ở các phần trên.

| Viết tắt | Nghĩa đầy đủ | Một câu |
|---|---|---|
| ABI | Application Binary Interface | Hợp đồng nhị phân giữa kernel/user. |
| ACL | Access Control List | Danh sách quyền chi tiết (chưa có trong dự án). |
| ACPI | Advanced Configuration and Power Interface | Chuẩn mô tả phần cứng, CPU, tiết kiệm điện. |
| AHCI | Advanced Host Controller Interface | Giao diện SATA hiện đại. |
| AP | Application Processor | CPU phụ trên x86. |
| API | Application Programming Interface | Giao diện lập trình. |
| ASID | Address Space ID | Nhãn TLB theo không gian địa chỉ. |
| ATA | Advanced Technology Attachment | Chuẩn đĩa cũ. |
| BKL | Big Kernel Lock | Khóa lớn toàn kernel. |
| BootInfo | Boot Information | Trang dữ liệu runtime trao cho root task. |
| BSP | Boot Strap Processor / Board Support Package | CPU khởi động / gói hỗ trợ board. |
| CDT | Capability Derivation Tree | Cây cha–con của capability. |
| CI | Continuous Integration | Build/test tự động. |
| CNode | Capability Node | Object chứa slot capability. |
| COW | Copy-On-Write | Sao chép khi ghi (chưa có trong dự án). |
| CPU | Central Processing Unit | Bộ xử lý. |
| CRC | Cyclic Redundancy Check | Checksum phát hiện lỗi. |
| CR3 | Control Register 3 | Thanh ghi root page table x86. |
| CSpace | Capability Space | Không gian chứa capability của thread. |
| CSR | Control and Status Register | Thanh ghi cấu hình RISC-V. |
| DMA | Direct Memory Access | Thiết bị tự đọc/ghi RAM. |
| DMAR | DMA Remapping Reporting | Bảng ACPI cho VT-d. |
| ECC | Error-Correcting Code | Mã sửa lỗi bộ nhớ. |
| ELF | Executable and Linkable Format | Định dạng file thực thi. |
| EL0/EL1/EL2/EL3 | Exception Levels | Cấp đặc quyền ARM. |
| ESP | EFI System Partition / Extended Stack Pointer | Phân vùng FAT UEFI / con trỏ stack. |
| fd | File Descriptor | Số đại diện file đang mở. |
| FIFO | First In, First Out | Vào trước ra trước. |
| FP | Floating Point | Số dấu phẩy động. |
| FS | File System | Hệ thống file. |
| FSCK | File System Check | Kiểm tra/sửa filesystem. |
| GDT | Global Descriptor Table | Bảng segment x86. |
| GIC | Generic Interrupt Controller | Bộ điều khiển ngắt ARM. |
| GID | Group ID | Số định danh nhóm người dùng. |
| GOP | Graphics Output Protocol | Framebuffer UEFI. |
| HAL | Hardware Abstraction Layer | Lớp trừu tượng phần cứng. |
| hart | Hardware Thread | CPU core theo thuật ngữ RISC-V. |
| HSM | Hart State Management | SBI quản lý hart. |
| IDT | Interrupt Descriptor Table | Bảng handler ngắt x86. |
| IMU | Inertial Measurement Unit | Cảm biến quán tính (demo). |
| inode | Index Node | Metadata của file. |
| IOMMU | I/O Memory Management Unit | MMU cho thiết bị. |
| IOVA | I/O Virtual Address | Địa chỉ thiết bị nhìn thấy. |
| IPC | Inter-Process Communication | Giao tiếp giữa tiến trình. |
| IPI | Inter-Processor Interrupt | Ngắt giữa các CPU. |
| IRQ | Interrupt Request | Yêu cầu ngắt. |
| ISA | Instruction Set Architecture | Tập lệnh CPU. |
| IST | Interrupt Stack Table | Stack riêng cho exception x86. |
| KResult | Kernel Result | `Ok(value)` hoặc `Err(KError)`. |
| KVM | Kernel-based Virtual Machine | Tăng tốc ảo hóa Linux. |
| LAPIC | Local Advanced Programmable Interrupt Controller | Bộ điều khiển ngắt cục bộ x86. |
| LBA | Logical Block Addressing | Địa chỉ block đĩa. |
| LTO | Link-Time Optimization | Tối ưu lúc link. |
| MADT | Multiple APIC Description Table | Bảng ACPI liệt kê CPU/LAPIC. |
| MCP | Maximum Controlled Priority | Ưu tiên tối đa được phép đặt. |
| MCS | Mixed Criticality Scheduling | Lập lịch theo mức quan trọng. |
| Miri | — | Công cụ phát hiện undefined behavior trong Rust. |
| MMIO | Memory-Mapped I/O | I/O qua địa chỉ bộ nhớ. |
| MMU | Memory Management Unit | Dịch VA → PA. |
| MOTD | Message Of The Day | File `/motd` chứa lời chào. |
| NX | No-eXecute | Cấm thực thi. |
| NVMe | Non-Volatile Memory Express | Chuẩn SSD. |
| OOM | Out Of Memory | Hết bộ nhớ. |
| OS | Operating System | Hệ điều hành. |
| OVMF | Open Virtual Machine Firmware | UEFI firmware cho QEMU. |
| PA | Physical Address | Địa chỉ vật lý. |
| PageTableOps | Page Table Operations | Trait đọc/ghi PTE, flush, zero frame, sync I-cache. |
| PAN | Privileged Access Never | Cấm kernel truy cập user memory. |
| PCI/PCIe | Peripheral Component Interconnect | Bus thiết bị phổ biến trên x86. |
| PCID | Process Context ID | ASID của x86. |
| PDPT | Page Directory Pointer Table | Cấp 2 page table x86. |
| PE/COFF | Portable Executable / Common Object File Format | Định dạng `.efi`. |
| PIC | Programmable Interrupt Controller / Position-Independent Code | Chip ngắt / code độc lập vị trí. |
| PIO | Programmed I/O | CPU tự đọc/ghi thiết bị (ATA PIO). |
| PIT | Programmable Interval Timer | Timer legacy x86. |
| PLIC | Platform-Level Interrupt Controller | Bộ điều khiển ngắt RISC-V. |
| PML4 | Page Map Level 4 | Bảng trang gốc x86. |
| PMP | Physical Memory Protection | Bảo vệ bộ nhớ vật lý RISC-V. |
| POSIX | Portable Operating System Interface | Chuẩn API kiểu Unix; dự án chưa tương thích. |
| PPI | Private Peripheral Interrupt | Ngắt riêng từng CPU ARM. |
| PSCI | Power State Coordination Interface | Giao diện quản lý CPU ARM. |
| PTE | Page Table Entry | Mục page table. |
| RAII | Resource Acquisition Is Initialization | Tự động nhả tài nguyên. |
| RPC | Remote Procedure Call | Gọi hàm từ xa (ở đây qua IPC). |
| RX | Read + Execute | Đọc và thực thi. |
| RW | Read + Write | Đọc và ghi. |
| RO | Read Only | Chỉ đọc. |
| SBI | Supervisor Binary Interface | Giao diện kernel–firmware RISC-V. |
| SchedContext | Scheduling Context | Object ngân sách CPU. |
| SGI | Software Generated Interrupt | Ngắt phần mềm ARM. |
| SIPI | Startup IPI | IPI đánh thức AP. |
| SMAP | Supervisor Mode Access Prevention | Cấm kernel đọc/ghi user memory. |
| SMEP | Supervisor Mode Execution Prevention | Cấm kernel chạy code user. |
| SMMU | System Memory Management Unit | IOMMU của ARM. |
| SMP | Symmetric Multiprocessing | Đa xử lý đối xứng. |
| SPI | Shared Peripheral Interrupt | Ngắt thiết bị dùng chung ARM. |
| SRST | System Reset | SBI reset/tắt máy. |
| Sv39 | 39-bit Virtual Memory Scheme | MMU 3 cấp RISC-V. |
| TCB | Thread Control Block / Trusted Computing Base | Object thread / tập tin cậy. |
| TCG | Tiny Code Generator | Bộ dịch động QEMU. |
| TCR | Translation Control Register | Cấu hình MMU ARM. |
| TLS | Thread Local Storage | Dữ liệu riêng của từng thread. |
| TLB | Translation Lookaside Buffer | Cache ánh xạ VA → PA. |
| Trampoline | — | Đoạn code đánh thức AP trên x86. |
| TSS | Task State Segment | Lưu stack kernel x86. |
| TTBR0/TTBR1 | Translation Table Base Register | Root page table ARM. |
| TSC | Time Stamp Counter | Bộ đếm chu kỳ x86. |
| UEFI | Unified Extensible Firmware Interface | Firmware chuẩn. |
| UID | User ID | Số định danh người dùng. |
| UserAction | — | Enum runtime mô tả việc kernel muốn runtime làm tiếp. |
| UXN/PXN | User/Privileged Execute Never | Cấm thực thi ARM. |
| VA | Virtual Address | Địa chỉ ảo. |
| VF04 | ViFast FS version 4 | Magic/phiên bản filesystem hiện tại. |
| VFS | Virtual File System | Lớp giao diện filesystem. |
| VirtIO | Virtual I/O | Chuẩn thiết bị ảo. |
| VM | Virtual Memory / Virtual Machine | Bộ nhớ ảo / máy ảo. |
| VSpace | Virtual Address Space | Không gian địa chỉ ảo. |
| VT-d | Virtualization Technology for Directed I/O | IOMMU Intel. |
| WAL | Write-Ahead Log | Ghi log trước. |
| WCET | Worst-Case Execution Time | Thời gian chạy xấu nhất. |
| WFI | Wait For Interrupt | Ngủ chờ ngắt. |
| W^X | Write xor Execute | Không vừa ghi vừa thực thi. |
| x2APIC | Extended x2APIC | LAPIC mở rộng. |
| XSDT | Extended System Description Table | Bảng ACPI mở rộng. |

---

## 11. Đọc log QEMU như thế nào?

Khi chạy `python3 scripts/test-qemu.py`, bạn sẽ thấy nhiều dòng log. Đây là
các dòng quan trọng:

| Dòng log | Ý nghĩa |
|---|---|
| `booting on riscv64` | Kernel đã vào được entry, console hoạt động. |
| `SELFTEST PASS` | Self-test trong kernel đã qua. |
| `SMP OK (harts=2, ipi=1)` | Hai hart đã online, IPI hoạt động. |
| `ACPI MADT OK` | x86 đã parse bảng ACPI, thấy CPU/LAPIC. |
| `LAPIC OK` | LAPIC đã được map/bật. |
| `ROOT RUST` | Root task Rust đã chạy. |
| `BOOTINFO OK` | Root đọc được BootInfo hợp lệ. |
| `ROOT READY` | Root đã tạo xong server/client/fault. |
| `CLIENT VSPACE OK` | VSpace riêng của client đã dựng. |
| `FAULT EP OK` | Fault endpoint nhận được fault message. |
| `Server` / `Client` | Các task user mẫu đã chạy. |
| `Mapped` | Client map frame và ghi/đọc thành công. |
| `Cap OK` | Chuyển capability qua IPC thành công. |
| `Reply OK` | Call/Reply hoạt động. |
| `ViFastOs ramfs` | Đọc file `/motd` từ ramfs. |
| `ViFastOs write` | Ghi/đọc file `/scratch`. |
| `VFS SERVER OK` | VFS server user-space đã chạy. |
| `VFS OK` | Client đọc/ghi qua VFS server thành công. |
| `FS DISK OK` | Filesystem trên đĩa thật hoạt động. |
| `FS PERSIST OK` | Dữ liệu còn sau khi reboot (test persistence). |
| `IOMMU OK` | Mô hình IOMMU phần mềm hoạt động. |
| `BLOCK OK` | Block driver đọc/ghi được. |
| `PREEMPT OK` | Timer preemption đã chạy. |
| `USER FAULT` | User cố tình gây lỗi; kernel không panic. |
| `ROOT SWITCH n` | Số lần chuyển VSpace/root. |
| `PERCPU CLOBBER OK` | Kiểm tra per-CPU state không bị ghi đè. |
| `SMP WORKERS OK` | Các worker chạy trên hart phụ. |
| `SMP PREEMPT OK` | Preemption trên nhiều hart. |
| `TLB REMAP OK` | TLB shootdown/remap hoạt động. |
| `BADGE SPOOF DENIED OK` | Không thể giả badge. |
| `TIME MONOTONIC OK` | Thời gian không lùi. |

Nếu thấy `ROOT PANIC`, `SELFTEST FAIL`, `VFS BAD`, `PERCPU CLOBBER BAD`…
thì đó là tín hiệu lỗi, không phải “tính năng”.

---

## 12. Lộ trình đọc repo cho sinh viên năm nhất

Đừng đọc hết `docs/` từ đầu đến cuối. Đọc theo thứ tự sau:

1. **`README.md`** — biết dự án là gì, chạy thế nào.
2. **`docs/00-hien-trang.md`** — biết cái gì đã có, cái gì chưa.
3. **`docs/11-hien-tai/00-index.md`** — bản đồ tài liệu thành phần.
4. **`docs/11-hien-tai/01-abi.md`** — syscall, quyền, message.
5. **`docs/11-hien-tai/02-objects-capabilities.md`** — object/cap/untyped.
6. **`docs/11-hien-tai/03-ipc-scheduler.md`** — IPC/scheduler.
7. **`docs/11-hien-tai/04-memory-vspace.md`** — MMU/VSpace/TLB.
8. **`docs/11-hien-tai/05-vfs.md`** — VFS/ramfs (một phần đã cũ; xem thêm
   `docs/ViFastOs-agent-handoff.md` và `ViFastOs-architecture-summary.txt`).
9. **`docs/11-hien-tai/06-runtime-platform.md`** — boot/runtime.
10. **`docs/11-hien-tai/07-arch-ports.md`** — ba port.
11. **`docs/11-hien-tai/08-boot-user-flow.md`** — luồng boot/user.
12. **`docs/11-hien-tai/09-testing.md`** — test.
13. **`docs/11-hien-tai/10-vifast-user.md`** — wrapper syscall.
14. **`docs/11-hien-tai/11-phase-3b-decisions.md`** — quyết định thiết kế.
15. **`docs/11-hien-tai/12-vifast-pte.md`** — PTE.
16. **`docs/ViFastOs-agent-handoff.md`** — trạng thái/roadmap mới nhất.
17. **`ViFastOs-architecture-summary.txt`** — bản tổng hợp chi tiết.

Sau đó mới mở code:

- `crates/vifast-abi/src/lib.rs` — “từ điển” kiểu dữ liệu.
- `crates/vifast-kernel/src/object.rs` — object/cap.
- `crates/vifast-kernel/src/ipc.rs` — IPC.
- `crates/vifast-kernel/src/lib.rs` — scheduler/syscall.
- `crates/vifast-pte/src/lib.rs` — page table.
- `crates/vifast-runtime/src/lib.rs` — boot/dispatch.
- `crates/vifast-platform-*/src/platform.rs` — phần cứng.
- `crates/vifast-fs/src/lib.rs` — filesystem.
- `crates/vifast-server/src/main.rs` — VFS server.

---

## 13. Mười hiểu nhầm thường gặp

1. **“Boot được = OS hoàn chỉnh.”** Không. Boot được chỉ chứng minh entry,
   console, page table, user mode… hoạt động. Còn driver, mạng, GUI, ứng dụng,
   bảo mật thực tế.
2. **“Microkernel tự động an toàn hơn.”** Không. Nó chỉ làm TCB nhỏ hơn, dễ
   kiểm chứng hơn; IPC và driver user-space lại thêm bề mặt lỗi.
3. **“Microkernel tự động nhanh hơn.”** Không. IPC có chi phí; muốn nhanh phải
   tối ưu fast path, zero-copy, batch.
4. **“TCB chỉ có một nghĩa.”** Không. Thread Control Block và Trusted Computing
   Base là hai nghĩa khác nhau.
5. **“BSP chỉ có một nghĩa.”** Không. Boot Strap Processor và Board Support
   Package.
6. **“PIC chỉ là chip ngắt.”** Không. Còn là Position-Independent Code.
7. **“VFS là filesystem.”** VFS là lớp giao diện; FS là cách lưu cụ thể.
8. **“Page table là RAM.”** Page table là cấu trúc dữ liệu trong RAM, dùng để
   dịch địa chỉ; không phải dữ liệu của ứng dụng.
9. **“W^X là phần mềm diệt virus.”** Không. Nó là quy tắc quyền bộ nhớ.
10. **“Test pass = chứng minh đúng.”** Không. Test chỉ kiểm tra những trường
    hợp đã viết; cần fuzz, invariant, formal verification.

---

## 14. Từ điển “một câu” cho các khái niệm khó

- **Capability**: vé không làm giả được, trỏ tới object + quyền.
- **Untyped**: đất thô chưa xây.
- **Retype**: xây object có kiểu trên đất thô.
- **Revoke**: thu hồi vé và mọi vé con.
- **Badge**: nhãn danh tính trên vé.
- **Endpoint**: quầy giao dịch IPC.
- **Notification**: đèn báo bất đồng bộ.
- **Fault endpoint**: chuông báo sự cố cho supervisor.
- **VSpace**: bản đồ địa chỉ ảo của một process.
- **Frame**: một trang vật lý.
- **PTE**: một dòng trong bản đồ địa chỉ.
- **TLB**: cuốn sổ tay cache bản đồ.
- **Shootdown**: gọi các CPU khác xóa sổ tay cũ.
- **W^X**: không cho trang vừa ghi vừa chạy.
- **Guard page**: trang trống cạnh stack để bắt tràn.
- **IPC buffer**: bàn chung để chuyển dữ liệu lớn.
- **Journal**: nhật ký ghi trước để phục hồi sau crash.
- **Inode**: chứng minh thư của file.
- **Hard link**: nhiều tên, một file.
- **Symlink**: tên trỏ tới đường dẫn khác.
- **fsync**: bắt dữ liệu xuống đĩa thật.
- **SMP**: nhiều CPU chạy chung một kernel.
- **IPI**: ngắt từ CPU này sang CPU khác.
- **BKL**: một khóa lớn cho cả kernel.
- **WCET**: thời gian xấu nhất.
- **MCS**: lập lịch theo mức quan trọng.
- **Formal verification**: chứng minh bằng toán, không chỉ test.

---

## 15. Kết luận: đọc dự án này để học gì?

ViFastOs là một dự án tốt để học vì nó chạm vào hầu hết trụ cột của hệ điều
hành:

- quyền truy cập (capability, least privilege);
- giao tiếp (IPC, notification, RPC);
- thời gian (scheduler, preemption, SMP, real-time);
- bộ nhớ (MMU, page table, TLB, W^X);
- lưu trữ (VFS, journal, crash recovery);
- phần cứng (ba kiến trúc, firmware, driver);
- kỹ nghệ phần mềm (Rust, test, fuzz, invariant, multi-arch).

Đừng cố hiểu hết ngay. Hãy chọn một nhánh — ví dụ capability — rồi đi từ
`vifast-abi` → `object.rs` → `ipc.rs` → test. Khi đã hiểu một nhánh, các nhánh
khác sẽ dễ hơn rất nhiều.

Nếu bạn chỉ nhớ được ba điều:

1. **Kernel làm ít, user space làm nhiều.**
2. **Capability là vé; không có vé thì không có quyền.**
3. **IPC là cái giá và cũng là sức mạnh của microkernel.**
