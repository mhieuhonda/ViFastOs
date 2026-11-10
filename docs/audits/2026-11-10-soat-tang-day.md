# Soát lại tầng đáy — kernel, platform, runtime, storage (2026-11-10)

Người soát: phiên audit độc lập (không sửa mã nguồn).
Mốc: `cc37c90` (r287) + 3 file dirty sẵn có (`vifast-kernel`, `vifast-runtime`,
`09-testing.md` — bản vá T33a chưa commit).
Trạng thái nền: `cargo test -p vifast-kernel` **243/243 PASS**; clippy
`--workspace --all-targets` **80 warning, 0 error**; `cargo test` cho
pte/elf/fs/fsutil/arch đều xanh.

Cách làm: đọc trực tiếp + 5 mũi soát song song theo tầng (sched/IPC, VM,
cap/untyped/destroy, platform/arch/ACPI/VirtIO, FS). Mỗi phát hiện dưới đây
ghi rõ ai kiểm chứng:

- **[T]** — tôi tự đọc mã hoặc tự tái hiện trong phiên này.
- **[S]** — mũi soát phụ đã tái hiện bằng chương trình nháp trong `/tmp`
  (repo không bị sửa); tôi đã đọc lại đoạn mã liên quan nhưng chưa tự chạy lại.
- **[S-đọc]** — mới suy ra từ đọc mã, chưa có đường tái hiện.

---

## 1. Trả lời ngắn

**Nền có chắc không?** Chắc hơn mặt bằng chung của một OS tự viết ở tuổi này:
245 test host, gate QEMU 3 kiến trúc, kỷ luật test-đỏ-trước (D18), bộ
`check_invariants` 492 dòng, và `vifast-kernel`/`vifast-runtime` là
`#![forbid(unsafe_code)]`. Nhưng "chắc" ở đây đang **không đều giữa các tầng**:

| Tầng | Có lưới kiểm chứng? | Nơi lỗi nặng nhất tập trung |
|---|---|---|
| `vifast-kernel` | **Có** — 245 test host + `check_invariants` + fuzz | lỗi còn lại là *bất biến chưa được phát biểu*, không phải code ẩu |
| `vifast-fs` | **Có một phần** — 40 test + `fsck` + test ảnh hỏng | biên thiếu (`block_is_used`), rollback thiếu, `fsck` kiểm RAM thay vì đĩa |
| `vifast-runtime` | **Có** — nhưng lẫn với harness test | state machine budget/sleep, `unwrap_or(0)` trên slot ngữ cảnh |
| `vifast-platform-*` | **KHÔNG có một test host nào** | **toàn bộ P0 của đợt này** |
| `vifast-arch` | Có 7 test (BKL) | BKL bị dùng như "đã giữ" trong khi thực ra chỉ "đang chờ" |

Điểm mấu chốt: **ba lỗi P0 đều nằm ở tầng duy nhất không có test host** —
`vifast-platform-*` (asm, MMIO, IPI, TLB). Đó không phải trùng hợp: tầng này
chỉ được kiểm gián tiếp qua marker boot trên QEMU, mà QEMU lại "dễ tính" hơn
phần cứng thật ở đúng ba chỗ: thứ tự IPI, hành vi TLB theo ASID, và độ trễ
giữa publish và consume.

**Trả lời câu "nền chắc thì xây cao mới tốt":** đúng, nhưng thước đo không
phải "hết bug" mà là **mỗi tầng có một hợp đồng và một phép kiểm có thể đỏ**.
Hiện 3/5 tầng có; tầng platform thì không. Vì vậy khoản đầu tư đúng tiếp theo
không phải thêm tính năng, mà là **làm cho tầng platform kiểm được** (mô hình
mailbox/TLB chạy host + canary + test 3 kiến trúc), rồi mới cắt cấu trúc.

---

## 2. Lỗi P0 — cách ly / an toàn bộ nhớ

### P0-1. aarch64: mọi leaf user đều là **global**, nên `aside1is` không xoá khi đổi VSpace
`crates/vifast-pte/src/lib.rs:236` — `encode_leaf` ghép `0b11 | AF | SH_INNER | ap | PXN | uxn`,
**thiếu bit 11 (nG)**. `crates/vifast-platform-aarch64/src/platform.rs:1303` đổi không
gian địa chỉ bằng `msr ttbr0_el1` + `tlbi aside1is, <asid>`. Theo ARM ARM,
`ASIDE1` **không** invalidate entry global (nG=0).

Hệ quả: hai VSpace cùng map một VA (chuyện có thật trong hệ này —
`client_vspace` và `server_vspace` đều map `CODE_VA`, khác PA, xem
`crates/vifast-root/src/main.rs:2578-2579`) thì entry TLB của VSpace trước
sống sót sang VSpace sau. Lần chuyển tiến trình đầu tiên có thể **chạy mã của
tiến trình khác**. [T] đọc mã + [S] phân tích.

Vì sao hôm nay vẫn xanh: đường boot cho root và server **dùng chung một
frame** ở `CODE_VA` (`build_task_space_shared_code`,
`crates/vifast-runtime/src/lib.rs:790`), nên entry "global" tình cờ đúng cho
cả hai; và QEMU có thể flush rộng tay hơn phần cứng thật. Đây là lỗi
"xanh trên QEMU, sai trên máy thật" đúng nghĩa.

Liên đới: `flush_va` (`platform.rs:902-908`) phát `tlbi vae1is` **không gắn
ASID** — hiện chỉ "có tác dụng" vì mọi leaf là global. Nếu ai đó vá nG mà
không vá chỗ này thì `flush_va` thành no-op. **Hai chỗ phải sửa cùng nhau.**

Vá: `| (1 << 11)` trong `encode_leaf` + test khẳng định bit nG; `flush_va`
phải tag `(asid << 48) | va` (hoặc dùng `vmalle1is`). Test đỏ-trước: encode
một leaf rồi assert `& (1<<11) != 0`; và một test "hai VSpace cùng VA khác PA"
ở mức `PageTableOps` giả.

### P0-2. Shootdown publish **seq trước** payload; target ACK việc nó chưa làm
Ba port đều làm: `SEQ.fetch_add` → `KIND.fetch_or` → doorbell
(`platform-aarch64:313-316`, `platform-riscv64:275-278`,
`platform-x86_64:1198-1202`). Consumer: đọc `SEQ` → nếu `ACK >= seq` thì
return → `KIND.swap(0)` → làm việc → `ACK.store(seq)` **vô điều kiện**
(`platform-aarch64:246-285`, `platform-riscv64:219-246`,
`platform-x86_64:1164-1182`).

Khe hở: target poll đúng giữa hai lệnh publish ⇒ thấy `seq` mới nhưng
`kind == 0` ⇒ vẫn `ACK.store(seq)`. Publisher thấy đủ ACK, coi như đã flush.
IPI đến sau bị `ACK >= seq` chặn. ⇒ **mất một lần invalidate TLB**.

Điều làm nó không hiếm: trên x86 hart phụ poll chính trong vòng quay BKL
(`platform-x86_64:272-279`) — đúng lúc hart chính đang giữ BKL để flush.
Trên aarch64/riscv, hart phụ cũng poll trong lúc chờ BKL.

Hệ quả: entry TLB cũ trỏ vào frame đã được trả về Untyped và cấp lại cho
tiến trình khác ⇒ đọc/ghi chéo vùng nhớ. [T] đọc mã 3 port.

Vá (một chỗ, dùng chung): đảo thứ tự — ghi payload, `KIND.fetch_or` (Release),
rồi `SEQ.fetch_add`, rồi barrier + doorbell; consumer chỉ `ACK` khi
`kind != 0` hoặc khi đã thấy `seq` mới **sau** khi đọc `kind`. Kèm
`dsb ish` / `mfence` / `fence rw,rw` trước doorbell (hiện thiếu ở cả ba).

### P0-3. Cap lạc: `cap.parent` trỏ TCB đã chết ⇒ revoke không tới, object thành bất tử
`reparent_tcb_caps` (`vifast-kernel/src/lib.rs:3942-3996`) **return sớm** nếu
TCB đang dùng một CNode (`:3943-3946`), vì "CNode là object độc lập". Nhưng
vòng lặp sửa `cap.parent` chỉ đi qua `cspace_mut` (`:3979-3995`), mà
`cspace_mut` (`:473-486`) chỉ với tới CNode **thông qua TCB đang dùng nó**.
CNode đã tháo khỏi TCB ⇒ không ai sửa `cap.parent` của nó.

Kịch bản: mint một cap vào CNode C từ slot của TCB A → `C.slots[2].parent`
trỏ `{tcb: A, slot: 2}`. Huỷ A ⇒ `cap.parent` là con trỏ chết. Sau đó
`revoke_cap` đi qua `is_descendant` (`:3851-3871`) và **bail ở bước nhảy
không phân giải được** ⇒ trả `Ok(0)` ⇒ cap sống. `destroy_cap_object`
(`:4820-4860`) lại thấy `other_caps_to` (`:4619-4652`) có tham chiếu ⇒
`HasDependents` ⇒ object không bao giờ huỷ được. `check_invariants` **không**
bắt được vì nó chỉ kiểm `cap.parent` trong nhánh `badge != 0`
(`:3235-3246`). [S] tái hiện; [T] đọc lại toàn bộ đường đi.

Vá: một **bộ duyệt cấu trúc duy nhất** trên mọi bảng cap vật lý (root cspace,
`Tcb.cspace`, **mọi** CNode dù đã tháo hay chưa) — hiện có bốn bộ duyệt độc
lập (`:3753`, `:3979`, `:4621`, `:5250`) và đó là gốc của cả P0-3 lẫn P2-2.

---

## 3. Lỗi P1 — chức năng / bền vững

| # | Lỗi | Vị trí | Hậu quả | Kiểm chứng |
|---|---|---|---|---|
| P1-1 | FS: `load_metadata` dùng `dev.block_count()` thay vì `total` của superblock; `block_is_used` thiếu chặn biên bitmap | `vifast-fs/src/lib.rs:940`, `:1141-1144` | Đĩa **lớn hơn** vùng đã format (4 MiB) ⇒ panic `index out of bounds: len 1024, index 1024` ngay khi mount | **[T] đã tái hiện**: `dd` 4 MiB → `mkfs` → `truncate -s 8M` → `stat` = panic |
| P1-2 | FS: `file_block` đọc bảng indirect bằng chỉ số không kiểm ⇒ OOB panic | `vifast-fs/src/lib.rs:1230,1238` → panic tại `:287` | Ảnh FS độc hại (inode size bịa) làm **halt** kernel thay vì trả `Corrupt` | [S] tái hiện cả VF03/VF04 |
| P1-3 | FS: ghi quá EOF tạo lỗ, không zero-fill | `vifast-fs/src/lib.rs:1802-1819` | File thành không đọc được (`Corrupt`) + `fsck` báo hỏng; client với tới được (server không kiểm offset) | [S] tái hiện |
| P1-4 | FS: `create` thất bại rò inode đã `stage_inode`, không rollback | `vifast-fs/src/lib.rs:1629-1637` | Đĩa có inode mồ côi; lần commit sau ghi nó xuống ⇒ `fsck` hỏng | [S] tái hiện |
| P1-5 | FS: `unlink` giải phóng inode đang mở, `alloc_inode` tái dùng ino | `vifast-fs/src/lib.rs:2119-2132`, `:1344-1357` | fd cũ đọc/ghi **file khác** — rò dữ liệu giữa hai file | [S] tái hiện |
| P1-6 | x86: AP không bao giờ shootdown BSP | `vifast-platform-x86_64/src/platform.rs:2061-2065` (`target != 1 → continue`), `:2082` (`me == 0`) | Hart 1 sửa bảng trang (nó chạy syscall) nhưng hart 0 giữ TLB cũ ⇒ đọc frame đã tái cấp | [T] đọc mã |
| P1-7 | Budget cạn làm chết hart | `vifast-kernel/src/lib.rs:5944` (`let _ = charge…`), `vifast-runtime/src/lib.rs:1731-1736` | `charge_current_budget` đặt `BlockedBudget` + `queued=false`, `yield_current_on` không enqueue được ⇒ `pick_on` = `None` ⇒ `A::exit(1)`. Đường `Sleep` đã được vá T33a, đường budget thì chưa | [T] đọc mã |
| P1-8 | `SECONDARY_ONLINE` là **bộ đếm**, dùng như **mask** | `platform-aarch64:294`, `platform-riscv64:255` | `>= target` chỉ đúng khi hart phụ lên đúng thứ tự 1,2,3. Boot hart 2 mà hart 1 chết ⇒ publish "thành công" vào hart không tồn tại, mỗi lần chờ 5M vòng | [T] đọc mã |
| P1-9 | ACPI: đọc RSDP offset 20 sau khi chỉ kiểm `len < 20`; MADT nhận `length >= 36` nhưng đọc `u32_at(madt,36)` | `vifast-acpi/src/lib.rs:79-92`, `:68`, `:116` | Firmware/RSDP 20–23 byte hoặc MADT 36–43 byte ⇒ panic lúc boot (DoS) | [T] đọc mã; `u32_at` index trực tiếp nên chắc chắn panic |
| P1-10 | `reply_epoch` không bao giờ tăng | `object.rs:620,657`; đọc ở `ipc.rs:360,376` | Toàn bộ guard `reply_epoch == epoch` của K49 là `0 == 0` — **rỗng**. Chỉ còn `state == BlockedReply` che | [T] `grep` toàn repo: không có chỗ nào `+=` |

---

## 4. Lỗi P2/P3 — tiềm ẩn, nợ nhỏ

| Lỗi | Vị trí | Ghi chú |
|---|---|---|
| `recv` trên bound-notification tiêu thụ `word` nhưng **không hạ `irq.pending`** (K46 mới vá `poll`) | `ipc.rs:248-260` vs `:568-590` | Cùng lớp lỗi K46; hiện chỉ test dùng đường này |
| `virtio` block: không kiểm `used.len`/`used.id`; timeout rồi dùng lại desc 0..2; DMA = **địa chỉ kernel ảo**; không đọc `QueueNumMax` | `vifast-virtio/src/lib.rs:1132-1192`, `:884-920` | Thiết bị lỗi/độc ⇒ trả 512 byte rác như dữ liệu đĩa |
| x86: #DF/NMI lấy BKL trước khi xem vector ⇒ `assert!(!held_by)` nổ trong chính handler #DF | `platform-x86_64:2901-2909`, `bkl.rs:98` | Mất khả năng chẩn đoán đúng lúc cần nhất |
| `tick()`: `stats.ticks = ticks[hart]` (hart cuối tick) và `system_ticks = fetch_max(...)` | `lib.rs:5915-5948` | Đồng hồ hệ thống đứng nếu hart nhanh nhất ngừng tick; `stats.ticks` không đơn điệu |
| `detach_tcb` không gọi `leave_sleep`/`leave_budget` | `lib.rs:3998-4039` | Bộ đếm `sleepers_active`/`budgeters_active` lệch (tự lành khi quét thấy 0) |
| `set_tcb_affinity` không kiểm `current` | `lib.rs:984-996` | Thu hẹp affinity của TCB đang chạy ⇒ hart không pick lại được |
| `wake_error` không gỡ khỏi endpoint/notification | `lib.rs:3873-3911` | Caller hiện tại đều pop trước; nếu có đường mới ⇒ sender kẹt `BlockedSend` |
| `wait_tcb` dùng chung `target.caller` | `ipc.rs:459-466` | Con đang `recv`/bị `Call` trả `WouldBlock` sai |
| `untyped_take`: `unaligned + align - 1` và `pa - base` không `checked_*` | `lib.rs:2034,2042` | Chỉ chạm khi base sát `u64::MAX`; nên siết cho rẻ |
| `enqueue` **im lặng bỏ** TCB Ready khi mọi hàng đầy | `lib.rs:5368-5371` | Bất khả thi *hôm nay* (MAX_TCBS=24 = dung lượng hàng) nhưng comment ghi "18" (sai) và không còn khe dự phòng |
| `register_staging` im lặng bỏ khi quá 32 | `lib.rs:606-615` | Rò frame, không hỏng |
| Trap stack: **16 KiB** trên x86/aarch64 nhưng **32 KiB** trên riscv; nửa mảng không dùng; **không canary** | `platform-x86_64:86,1706,1713`; `platform-aarch64:67,549`; `platform-riscv64:626` | Tràn stack sẽ ghi đè stack hart bên cạnh, im lặng |
| `StackSpec.guard` bị bỏ qua (`let _ = stack.guard;`) | `lib.rs:2777` | Guard page không được bảo đảm |
| `MAX_MAPPINGS = 64` + 1 Frame/trang ⇒ ELF > ~60 trang không exec được | `object.rs:29`, `lib.rs:2728` | Vách chức năng im lặng |
| `build_task_space` chấp nhận ảnh rỗng ⇒ entry không map | `lib.rs:2574` | Nên trả lỗi sớm |
| `check_invariants` (492 dòng, ~8 KiB biến cục bộ) nằm trong crate phát hành dù chỉ test gọi | `lib.rs:3164`, call site toàn bộ trong `mod tests` | Nên `#[cfg(any(test, feature = "check-invariants"))]` |
| `gate.sh` chặn sàn test ở **138** trong khi thực tế **245** | `scripts/gate.sh:47` | Xoá 107 test vẫn xanh — lưới an toàn có lỗ |
| clippy 80 warning (13 `is_multiple_of`, 10 cast thừa, 7 `div_ceil`, 4 assert hằng vô nghĩa trong `vifast-pte:416-422`) | toàn workspace | Nên `-D warnings` sau khi dọn |
| ramfs trong kernel + 6 syscall `Open/Read/Write/Close/Stat/Seek` **không chương trình nào gọi** | `vifast-kernel/src/fs.rs`, `vifast-abi:1027-1033` | Toàn bộ I/O thật đi qua VFS server bằng IPC (`vifast-client:140-151`) |

---

## 5. Nợ cấu trúc — cái gì tách được, số đo cụ thể

### 5.1 `vifast-kernel/src/lib.rs` = 16 328 dòng
Trong đó **7 295 dòng mã** + **8 885 dòng test** (55%). Một `impl Kernel` duy
nhất, 128 phương thức `pub`, ~220 hàm. Hàm dài nhất: `user_syscall_on`
**1 401 dòng** (6008–7408) — 20% toàn bộ mã không-test nằm trong một hàm.

Phân bố theo chủ đề (đo bằng cách gom tên hàm):

| Module đích | Hàm | Dòng | Ghi chú |
|---|---|---|---|
| `syscall/` | 2 | **1 450** | 58 syscall trong chuỗi `match n if n == SyscallId::X` |
| `vspace.rs` | 56 | 1 318 | + `task_space.rs` ~400 (builder) |
| `sched.rs` | 44 | 1 099 | gồm sleep/budget/suspend/resume/restart |
| `untyped.rs` | 23 | 723 | `retype_as` 183 dòng |
| `cspace.rs` | 19 | 645 | `revoke_cap` 120, `delete_cap` 68 |
| `destroy.rs` | 14 | 540 | `destroy_object` 221 |
| `invariants.rs` | 1 | 492 | `#[cfg(test)]` |
| còn lại (types, object table, create/reset) | — | ~1 000 | ở lại `lib.rs` |

Ranh giới kỹ thuật: mọi thứ đã là `impl Kernel` với field `pub(crate)`, nên
tách **chỉ là chuyển khối `impl Kernel { … }` sang file khác** + nâng ~30
helper private thành `pub(crate)`. Không đổi API ngoài crate, không đổi hành
vi. 245 test host là lưới cho việc này.

Hai chỗ đáng ưu tiên hơn cả vì chúng *sinh ra lỗi*:
1. **Bốn bộ duyệt bảng cap độc lập** (`:3753`, `:3979`, `:4621`, `:5250`) —
   gốc của P0-3 và P2-2. Gộp thành một iterator cấu trúc.
2. **Hai bản sao chính sách flush** (`flush_vspace_range` `:3090` và bản
   inline trong `map_page_table` `:2845-2861`) — nơi một lần sửa sai target
   sẽ ẩn.

### 5.2 Dispatch syscall: `match` bằng guard
`match number { n if n == SyscallId::Send as u64 => … }` (6138–7408) không
thể sinh jump table: **58 phép so sánh cho mỗi syscall**, và nhánh
`_ => NotImplemented` (7401-7404) làm trình biên dịch **không kiểm được tính
đầy đủ** — thêm một `SyscallId` mới sẽ biên dịch im lặng và trả
`NotImplemented` lúc chạy.

Vá cấu trúc: `impl TryFrom<u64> for SyscallId` + `match id { … }` (jump table
+ exhaustive) + mỗi nhóm syscall một `fn handle_x(&mut self, …) -> KResult<UserAction>`.
Kèm theo: đổi `slot_or_invalid!` (macro `return` thẳng từ hàm) thành `?` để
handler tách ra được.

### 5.3 Ba crate platform: 7 491 dòng, trùng lặp nặng
Đo trên dòng đã chuẩn hoá (bỏ comment/khoảng trắng), phần giao:

| Cặp | Dòng trùng / dòng file nhỏ |
|---|---|
| aarch64 ↔ riscv64 | **1 349 / 1 852 = 73%** (và 1 373/1 833 = 75% chiều ngược) |
| x86_64 ↔ aarch64 | 920 / 1 852 = 50% |
| x86_64 ↔ riscv64 | 1 218 / 1 833 = 66% |

23 hàm cùng tên ở cả ba: `poll_maintenance`/`process_maintenance`,
`publish_maintenance`, `wait_maintenance`, `send_maintenance`,
`shootdown_va`, `shootdown_all`, `maintenance_target_online`, `cpu_state`,
`publish_current_tcb`, `active_root`/`active_asid`/`set_active_root`,
`assert_current_tcb`, `assert_bkl_released`, `bkl_lock`, `init_cpu`,
`start_smp`, `build_kernel_page_tables`, `kernel_leaf_ok`, `zero_frame`…

Đề xuất `vifast-platform-common` (no_std, không asm) sở hữu:
- `MaintMailbox { seq, kind, ack, addr, pages, asid }` — **một** thứ tự
  publish đúng (vá luôn P0-2), `shootdown(&[hart]) -> UnackedMask`;
- `trait Doorbell { fn notify(hart); fn poll(); }` để mỗi port chỉ cấp
  ICR / SGIR / CLINT MSIP;
- `PerCpuState` + `publish_current_tcb`/`active_root`/`assert_current_tcb`;
- wrapper BKL + `assert_irqs_disabled`;
- scaffolding SMP: **online dưới dạng `AtomicU32` mask** (vá P1-8),
  `wait_stage`, đếm POST/IDLE.

Giữ lại theo port: chuỗi lệnh invalidate (`invlpg`+CR3 / `tlbi`+`dsb`+`isb`
/ `sfence.vma`), lập trình doorbell, trap entry + đổi stack, GDT/TSS/IDT +
trampoline + IST, encode/decode PTE + split/NX, PCID/ASID, console.

Ước tính: bớt ~600–700 dòng trùng, và quan trọng hơn: **ba bản sao của một
giao thức đồng bộ trở thành một bản**.

### 5.4 `vifast-runtime`: `Platform` 62 method, harness test nằm trong đường nóng
`Platform` (`lib.rs:83-318`) có 62 method, 32 có default. Trong đó **21 method
chỉ phục vụ marker chẩn đoán của boot**: `secondary_wakeups`,
`secondary_timer_ticks`, `secondary_user_ticks`, `secondary_preempts`,
`secondary_idle_entries`, `timer_rate`, `ipi_count`, `tlb_stats`,
`kernel_storage_canary`, `boot_stack_usage`, `secondary_none_wakes`,
`secondary_wait_blocked`, `post_idle_stats`, `stall_detail`, `post_idle_ready`,
`boot_handoff_from`, `user_entries`, `bkl_stats`, `resched_sent`,
`resched_wake`, `idle_entries`.

`timer::on_timer` (1491-1746) có ~150 dòng là `#[cfg(feature = "test-exit")]`
in marker `PREEMPT OK`/`SMP IDLE STALL`/`SMP AP STATE`; `boot()` 546 dòng
(490-1036) trộn bootinfo + ELF + VSpace + VFS + self-test.

Đề xuất: tách `bootcheck.rs` (toàn bộ marker + static `TEST_*`) sau feature
`diag`, chia trait thành `PlatformCore` (~25 method) / `PlatformSmp` (~10) /
`PlatformDiag` (~21, chỉ biên dịch khi `feature = "diag"`). `on_timer` còn
~30 dòng.

### 5.5 `vifast-fs`: 3 633 dòng = ~2 260 mã + ~1 370 test, một module phẳng
Tách được: `ondisk` (crc32, get/put, encode/decode inode, superblock — thuần
trên mảng byte, fuzz được ngay), `cache` (`CachedDev`, quy tắc write-through
`lba < DATA_START` nên thành tham số khởi tạo), `journal` (`stage`/`commit`/
`replay`), `fsck` (nên nhận `(SuperInfo, &[Inode], &[u8], &mut D)` thay vì
`&mut self` — sửa luôn P2 "fsck kiểm RAM thay vì đĩa"). Gắn thiết bị:
`read`/`write`/`truncate`/`lookup_path`/`write_dir_entry`.

---

## 6. Cái đã chắc — đừng phá khi refactor

Đã kiểm và **không** tìm thấy lỗi (ghi lại để vòng sau khỏi soát lại):

- Toán level/index của walker 3/4 cấp; `missing_table_level` khớp
  `ops.index`/`table_span`; rollback `build_task_space*` không huỷ bảng có sẵn.
- `mapped_w`/`mapped_x` được `check_invariants` tính lại từ reverse map và so
  sánh ⇒ lệch sẽ đỏ ngay. Đây là lý do các lỗi W^X không lọt.
- `vifast-elf`: kiểm overlap, đồng dư trang, W^X, entry nằm trong segment
  executable, `memory_end <= user_end` — đúng và có test.
- `has_ready_on` ↔ `pick_on` đã đồng bộ sau bản vá T33a (đang dirty trong
  working tree) — đúng hướng, giữ nguyên.
- `untyped_take`/`untyped_reclaim`: mọi đường lỗi sau khi đã lấy byte đều
  hoàn lại; free-list có coalesce + self-heal `leaked`.
- `destroy_object` kiểm lại mọi điều kiện của `can_destroy` (không tin bản
  trước) — đúng.
- `reply` khôi phục `caller` khi tra cap lỗi (r242) — đúng, có test đỏ-trước.
- Đường block của runtime: decode syscall kiểm cap + `validate_user_range`
  trước khi chạm đĩa — đúng.
- BKL: ticket lock FIFO, `holders` chống race #13, guard `#[must_use]`, test
  host 7 cái. (Chỉ có `is_held()` là "đang chờ" chứ không phải "đang giữ" —
  xem P2.)
- `vifast-kernel`/`vifast-runtime` không có `unsafe`, không `unwrap` ngoài 3
  chỗ đã được chặn bằng `is_some()`.

---

## 7. Thứ tự đề xuất (6 vòng, mỗi vòng một test đỏ-trước)

| Vòng | Nội dung | Vì sao trước |
|---|---|---|
| **r289** | P0-1 (nG + flush_va tag ASID), P0-2 (đảo publish + barrier), P1-6 (x86 đối xứng) | Ba lỗi cách ly; sửa ít dòng nhất, rủi ro thấp nhất, chặn "xanh QEMU / sai máy thật" |
| **r290** | P1-1 (capacity + biên bitmap), P1-3, P1-4, P1-5 (rollback + open-count), P1-2 (`slot()` trả `Option`) | Toàn bộ nằm trong `vifast-fs`, sửa được mà không đụng kernel |
| **r291** | P0-3 (một iterator cấu trúc cap), P1-10 (`reply_epoch` tăng ở restart), P1-7 (budget như T33a) | P0-3 là lỗi cách ly còn lại; sửa kèm bộ duyệt sẽ dọn luôn P2-2 |
| **r292** | Lưới an toàn: sàn test 138→245, canary trap stack, `cfg(test)` cho `check_invariants`, clippy `-D warnings`, P1-8/P1-9 | Rẻ, và làm mọi vòng sau an toàn hơn |
| **r293** | Cắt `lib.rs` → 7 module (cơ học, không đổi hành vi) | Phải sau khi lưới an toàn đã chắc |
| **r294** | `TryFrom<SyscallId>` + handler theo nhóm; `platform-common` + chia `Platform` | Hai refactor lớn, cần nền đã sạch |

Nguyên tắc: **không vòng nào vừa sửa lỗi vừa refactor**. Các vòng r289–r292
chỉ sửa; r293–r294 chỉ dời chỗ.

---

## 8. Phụ lục — tái hiện nhanh

```sh
# P1-1: panic khi mount đĩa lớn hơn vùng đã format
cd /tmp && dd if=/dev/zero of=big.img bs=512 count=8192
/Users/mps/dvchd/ViFastOs/target/debug/vifast-fsutil mkfs big.img
truncate -s 8M big.img
/Users/mps/dvchd/ViFastOs/target/debug/vifast-fsutil stat big.img /motd
# → panicked at crates/vifast-fs/src/lib.rs:1143:
#   index out of bounds: the len is 1024 but the index is 1024

# Sàn test của gate
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/mps/dvchd/ViFastOs && cargo test -p vifast-kernel -- --list | grep -c ': test$'
# → 245, trong khi scripts/gate.sh:47 chỉ đòi >= 138
```
