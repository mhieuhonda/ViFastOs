# 07. Thuật ngữ

| Thuật ngữ | Giải thích |
|---|---|
| ASID | Address Space ID. Nhãn gắn vào mục TLB để không phải xoá TLB khi đổi tiến trình |
| Badge | Số gắn vào cap endpoint/notification khi Mint; người nhận thấy được, dùng để phân biệt người gửi |
| BKL | Big Kernel Lock. Một khoá cho toàn bộ kernel trên máy nhiều lõi |
| Bound notification | Notification gắn với TCB, làm Recv trên endpoint trả về khi có tín hiệu |
| Capability (cap) | "Chìa khoá" không giả mạo được: tham chiếu tới object + quyền |
| CDT | Capability Derivation Tree. Cây cha con giữa các cap, dùng để revoke |
| CNode | Object chứa mảng slot, mỗi slot giữ một cap |
| cptr | Số 64 bit chỉ tới một slot trong CSpace |
| CSpace | Cây CNode của một thread |
| DMA | Thiết bị đọc/ghi bộ nhớ trực tiếp, không qua CPU |
| Endpoint | Object điểm hẹn IPC đồng bộ |
| Fault handler | Tiến trình nhận message khi thread khác bị lỗi |
| Guard | Dãy bit phải khớp khi phân giải cptr qua một CNode |
| Hart | Hardware thread (lõi) trong thuật ngữ RISC-V |
| IOMMU | Bộ dịch địa chỉ cho thiết bị, giới hạn vùng DMA |
| IoSpace | Bảng dịch IOMMU của một thiết bị |
| IPI | Inter-Processor Interrupt. Ngắt gửi từ lõi này sang lõi khác |
| MCP | Maximum Controlled Priority. Ưu tiên cao nhất một thread được phép đặt |
| MCS | Mixed Criticality Scheduling. Lập lịch có ngân sách thời gian |
| MMIO | Thanh ghi thiết bị truy cập như bộ nhớ |
| Notification | Object word bit cho báo hiệu bất đồng bộ |
| ObjRef | Tham chiếu object = chỉ số + generation |
| Pager | Tiến trình xử lý page fault của tiến trình khác |
| Preemption point | Điểm trong thao tác dài mà kernel có thể dừng và tiếp tục sau |
| PTE | Page Table Entry |
| Retype | Biến một phần Untyped thành object có kiểu |
| Revoke | Xoá mọi cap con cháu của một cap |
| SchedContext | Object chứa ngân sách CPU (budget/period) |
| Shootdown | Xoá mục TLB trên các lõi khác sau khi đổi page table |
| Supervisor | Tiến trình chính sách: nhận fault, quyết định restart |
| Sv39 | Chế độ bộ nhớ ảo 39 bit, 3 cấp của RISC-V |
| TCB | Thread Control Block. Cũng dùng với nghĩa Trusted Computing Base (phần phải tin tưởng) tuỳ ngữ cảnh |
| TLB | Bộ đệm dịch địa chỉ trong CPU |
| TrapFrame | Bản lưu thanh ghi user khi vào kernel |
| Untyped | Vùng nhớ thô chưa có kiểu, nguồn của mọi object |
| VSpace | Không gian địa chỉ ảo (page table gốc + ASID) |
| W^X | Write xor Execute: một trang không vừa ghi vừa thực thi |
| WCET | Worst-Case Execution Time |
| Zombie CNode | CNode đã hết cap trỏ tới, đang được dọn dần |
