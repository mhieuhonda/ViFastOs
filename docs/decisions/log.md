# Nhật ký quyết định tự động của agent2

| Vòng | Quyết định | Lý do | Commit |
|---|---|---|---|
| 26 | Revert A5/A1 khi QEMU đỏ, giữ baseline | Quy tắc kẹt 2.5 | — |
| 26 | Sửa display design theo C4 | T6: capability-safe device memory | 446a5c3 |
| 27 | Làm M0.2 (C1/C2/C3) trước M1.1 | Rủi ro thấp, docs đúng trước refactor | 91296ee,e69230e,d793786 |
| 27 | M1.1 chỉ commit phần đo, HWM sang inbox | AArch/x86 crash 3 lần | 216035d |
| 28 | M1.2 tách bước a/b/c, sửa runtime context indexing | ROOT ERR 16 do runtime dùng tcb.index | 91f7496 |
| 28 | HWM thử lần 4 (runtime read-only) vẫn lỗi AArch → BLOCKED, đi M1.3 | Quy tắc 2.5: 2 phiên BLOCKED thì đổi nhánh | — |
| 28 | Thử tách TCB pool, revert vì AArch 0/3 | Baseline xanh ưu tiên; lỗi AArch cần debug riêng | — |
| 29 | Wrap CPU_STATES + KERNEL_STORAGE AArch bằng UnsafeCell, không đổi hành vi | Giảm static mut, QEMU 3/3×3 + check-all xanh | d266ab7,36a5b91 |
| 29 | Stash CNode pool WIP, chỉ commit layout probe | CNode pool AArch 0/3; tách riêng để giữ baseline xanh | stash@{0},6682fa5 |
| 30 | Root cause tràn .data; guard KERNEL_STATIC_BUDGET; revert pool | Kernel pool đè KERNEL_PTR; exit rõ thay vì exception mù | (vòng này) |
| 31 | Giảm MAX_CAPS_TASK 64→48 sau audit slot | User max 11, test tới 36; tiết kiệm 150 KB Kernel | d6272f1 |
| 32 | Budget dùng chung kernel/runtime; mở goal dài hạn | Một ngưỡng duy nhất, tránh lệch; pha 1 M1→M3 | 25ac72c |
| 33 | Check tràn static AArch64 trong build | nm đo storage vs BKL; guard 2 lớp với runtime | 18772fa |
| 34 | Pager evict/unmap + fault lại (pha 2 bước 1) | Không tốn slot; PAGER3 stash do hết slot root | 9675b6e |
| 35 | Pager retype frame mới từ Untyped con meta (pha 2 bước 2) | Data chật x86; vòng đời khép kín qua pager | 1212b4d |
| 36 | Nâng MAX_CAPS_ROOT 96→128 (mở đường PAGER3/VFS) | Hết slot root; tăng root rẻ hơn task (+2 KiB) | 1810227 |
| 37 | PAGER3 client thứ ba (pha 2 bước 3) | Gộp stash vòng 34; 8 fault, harness 483 | e017792 |
| 38 | Tổng kết pha 2 ở MVP, hoãn demand-load VFS | Thiếu endpoint/IPC/slot; tránh rủi ro khi slot chật | cd4bb01 |
| 39 | VirtIO scan API + log device mỗi slot (pha 3 bước 1) | Chuẩn bị gắn net; hiện chỉ blk | b3b06bf |
| 40 | Giữ scan API, tắt log MMIO sau init (treo QEMU) | Đọc sau claim treo; probe trước init ở vòng sau | b1e39f3 |
| 41 | Xác minh treo MMIO, giữ nguyên code (docs-only) | Net hiện 1 lần rồi treo; serial test khác đường đọc | cce8546 |
| 42 | Probe net trước init + gắn virtio-net cả 2 arch | Serial đúng ở $TMPDIR; riscv net 0x10007000, arm 0xa003c00 | 66de790 |
| 43 | Log MAC net cả 2 arch qua net_mac trước init | Cả hai mac=52:54:00:12:34:56; config-space đọc tốt | 8fc0ae8 |
| 44 | Đọc feature bits net read-only, log kèm MAC | feats=0x30bf8024 đủ cho MVP 1 queue pair | d68f416 |
| 45 | NetQueue static riêng ngoài Kernel, log addr/size | Khe hở AArch64 giữ 8B; .bss sau BKL còn chỗ | b7bb020 |
| 46 | net_init_tx gắn queue TX, accept MAC/STATUS/VERSION_1 | Cả hai READY accepted=0x10020; RX để sau | e88bcdb |
| 47 | net_send_arp gửi ARP who-has gateway qua TX | Cả hai sent len=0 (QEMU consumed); RX reply sau | 774130b |
| 48 | Thêm RX API, hoãn gắn sau DRIVER_OK (QEMU kill rc=-9) | Giữ TX-only; vòng sau net_init_both | 33ef7a0 |
| 49 | net_init_both gắn RX+TX trước DRIVER_OK; bound RX 2M | Treo do poll 20M>20s, không phải QEMU kill | 4afaf71 |
| 50 | ARP 60B + peek chẩn đoán, xác nhận không có reply | Backend user-net cần kiểm tra ngoài guest | 851c7bc |
| 51 | Backend sống (monitor), chốt pha 3 MVP docs-only | Chuỗi probe→TX kiểm chứng; reply để sau | faf82b4 |
| 52 | Gắn ramfb + probe fw_cfg read-only cả 2 arch | sig=QEMU cả hai; sẵn đọc etc/ramfb | 4ed3ac1 |
| 53 | Ghi selector fw_cfg treo guest, revert sig-only | MMIO không có signature cố định; thử DMA/DTB sau | 6451851 |
| 54 | Probe DTB magic trong init_platform (runtime cấm unsafe) | RISC-V đúng, ARM cần map DTB | 9c2580b |
| 55 | Header FDT OK, quét struct treo, giữ header-only | Parse sau MMU ổn định hoặc copy DTB | 7d39f97 |
| 56 | Log đủ header FDT v17 trong vùng an toàn | strings_off=5716; sẵn copy DTB parse sau | dec123c |
| 57 | Tổng kết pha 4 ở hạ tầng probe, docs-only | Parse sâu để vòng sau (user-space/sau MMU) | e97bf13 |
| 58 | Dump opt-in chứng minh ARP TX lên dây | Pcap 60B đúng chuẩn; reply là backend/timing | cf7b6a1 |
| 59 | Khóa NET READY vào harness riscv64+aarch64 | Exact-one, fixture đúng base; pha 3 có test bảo vệ | b795c2c |
| 60 | Fmt 3 file + đánh giá M1-M3 trung thực | MVP đạt, mục mở có lý do, goal giữ active | 53a4dd6 |
| 61 | SHELL READY exact-one cả 3 arch | Input task có test bảo vệ; input thật để sau | 6102104 |
| 62 | `--interactive` gõ `help` qua chardev socket (opt-in) | riscv/arm 4/4; 3 lỗi AF_UNIX/-serial/PL011 LF đã sửa | 80e424c |
| 63 | Net TX+RX end-to-end: ping 10.0.2.2 nhận reply | 3 bug gốc: header 10B, QueueReady=1, RX kick/re-post | 55a2b0b |
| 64 | UDP + DNS hai chiều; DNS sent thành exact-one | Reply best-effort để gate không phụ thuộc DNS host; sửa re-post RX | cd82b4a |
| 65 | fw_cfg chỉ đọc trong môi trường này; display đi virtio-gpu | Kết quả âm có căn cứ + test checksum vào gate (204→207) | 72a0f9b |
| 66 | Display MVP qua virtio-gpu 160x120, pixel xác nhận bằng screendump | fw_cfg chặn hẳn; framebuffer lớn vượt headroom nên giảm kích thước | c8274e3 |
| 67 | Demand-load: nội dung trang nạp lúc fault | x86 hết Untyped nên chỉ bật theo target_arch; harness nới luật fault vì evict race | 0be7e59 |
| 68 | DNS A parser: guest resolve example.com ra IP thật | Marker DNS A best-effort (phụ thuộc DNS host), DNS sent vẫn cứng | ff37780 |
| 69 | Demand-load cả 3 arch: server tự retype frame lúc fault | Không cần root cấp frame; harness bỏ loại trừ x86 (495 cases) | 845b11d |
| 70 | virtio-input (device 18) đưa phím vào RxQueue UART, shell cũ dùng được | Test yêu cầu mã phím thô thay vì gõ lệnh (guest sống ngắn); đảo GPU trước net để hết flake | c61e3f1 |
| 71 | Demand-load nạp /page.bin từ VFS lúc fault (riscv/arm) | x86 giữ ảnh tĩnh; server sinh file theo chunk để không phình ELF | 694a7b2 |
| 72 | Thử x86 VFS demand-load qua boot Untyped — không khả thi | Thiếu cả frame lẫn PageTable; hoãn tới khi có cách giải phóng staging/untyped | (revert) |
| 73 | Boot splash: font 5x7 vẽ "ViFastOs" trên framebuffer | Verify bằng pixel chữ + hộp tối trong --gpu-check; ảnh PNG lưu docs/assets | 88a4a29 |
| 74 | `scripts/gate.sh` chạy toàn bộ cổng bằng một lệnh | Dùng cho các vòng liên tục và soak; `--quick` bỏ QEMU/persist | 6af1885 |
| 75 | virtio-rng (device 4) đọc entropy thật, 2 mẫu khác nhau | Harness khóa `VIRTIO RNG READY`; QEMU thêm virtio-rng-device | 5a09244 |
