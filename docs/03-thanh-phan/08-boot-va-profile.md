# 03.8. Boot, manifest và hai profile

Mã nguồn: `vifast-sim/src/boot.rs`; manifest `rust/manifests/*.toml`; Python `boot.py`.

## 1. Vai trò của boot

Boot (hiện ở simulator, sau này là root task hoặc công cụ build image) đọc manifest TOML và dựng toàn bộ hệ thống ban đầu:

1. CNode boot (4096 slot) chứa cap gốc: Untyped toàn bộ RAM (trừ 64 KB kernel), IrqControl, SchedControl.
2. Thiết bị: frame MMIO + IoSpace cho mỗi thiết bị.
3. Object dùng chung (`[objects]`): endpoint, notification, frame.
4. Mỗi task: CNode (mặc định 64 slot), TCB, VSpace (hoặc dùng chung với task khác), `ram_pages` trang RAM tại `0x100000`, các cap theo `caps`, fault endpoint, bound notification, SchedContext nếu có `budget/period`.
5. Đưa các task có `start != false` vào hàng đợi.
6. Profile `space`: xoá cap SchedControl, IrqControl, Untyped gốc.

## 2. Cú pháp manifest

```toml
profile = "space"        # hoặc "desktop" (mặc định)
memory = 1048576         # byte RAM cho untyped
harts = 2
iommu = true

[[devices]]
name = "camera"
kind = "dma_sensor"
mmio = 0x10000000
irq = 7

[objects]
fault_ep = "endpoint"
shared_buf = "frame"

[[tasks]]
name = "payload"
image = "payload"        # tên image đã đăng ký
priority = 120
mcp = 120                # mặc định = priority
core = 1                 # hart
budget = 3               # ngân sách CPU (tuỳ chọn)
period = 10
ram_pages = 1
cnode_bits = 6
start = true
share_vspace = "khác"    # dùng chung VSpace với task khác
bind_notification = "ntfn"
fault = { obj = "fault_ep", badge = 12 }
env = { crash_after = 6 }
caps = [ ... ]
```

Các dạng `caps` (mỗi dòng có `name` và `slot`):

| Khoá | Ý nghĩa |
|---|---|
| `obj = "x", rights = "RW", badge = n, map = vaddr` | Cap tới object dùng chung; frame có `map` thì map luôn |
| `device = "camera", rights = "RW", map = vaddr` | Frame MMIO của thiết bị |
| `iospace = "camera"` | IoSpace của thiết bị |
| `tcb = "task"`, `cnode = "task"`, `vspace = "task"` | Cap tới TCB/CNode/VSpace của task khác |
| `self = "vspace"` (hoặc tcb, cnode) | Cap tới của chính mình |
| `irq = 5, notification = "ntfn"` | IrqHandler, gắn sẵn notification |
| `untyped_bits = 16` | Untyped riêng 2^16 byte (chỉ desktop) |
| `irq_control = true`, `sched_control = true` | Quyền tạo IRQ/cấu hình ngân sách (chỉ desktop) |

## 3. Env của task

Chương trình nhận `env` (tên → số):
- `ram_base`, `ram_pages`
- Mỗi cap: `name → slot`; cap có `map`: thêm `name_addr → vaddr`
- `img_<tên>` → mã image (để tạo tiến trình lúc chạy)
- Các khoá trong `env = {...}`

Trên phần cứng, env sẽ là một trang "boot info" map sẵn vào mỗi task (giống seL4 BootInfo), hoặc hằng số sinh lúc build.

## 4. Profile desktop và space

Cả hai dùng chung kernel và chung cú pháp. Khác biệt chỉ ở quyền được phát:

| | Desktop | Space |
|---|---|---|
| `untyped_bits`, `irq_control`, `sched_control` trong caps | Cho phép | Lỗi boot |
| Cap gốc sau boot | Giữ trong CNode boot | Xoá |
| Hệ quả | Init tạo tiến trình, pager cấp trang, nhận thiết bị mới | Không ai tạo được object mới; tập tiến trình, bộ nhớ, ngắt, ngân sách cố định |

Test `space_is_static` duyệt mọi CNode để chứng minh không còn cap Untyped/IrqControl/SchedControl nào. Test `space_rejects_untyped` chứng minh manifest space không thể phát untyped.

Đây là hiện thực của ý tưởng "viết bản động trước, khoá lại thành bản tĩnh".

## 5. Demo

**space.toml (2 hart):**
- hart 0: supervisor (250), imu_driver (200, bound notification, IRQ 5), attitude (150, Call IMU 40 vòng)
- hart 1: payload (120, budget 3/10, panic sau 6 lệnh), producer/consumer (80, bộ nhớ chia sẻ + notification)
- Kết quả: attitude hoàn thành; payload bị restart 3 lần rồi DEGRADED; consumer nhận 3 khung.

**desktop.toml (2 hart, IOMMU):**
- console, init (tạo `hello` lúc chạy), pager + lazy (cấp trang theo yêu cầu), cam_driver (DMA qua IOMMU 3 khung).

## 6. Điểm chú ý
- Boot chạy với toàn quyền và không bị kiểm tra như syscall. Lỗi manifest được báo ra `BootError`, không được để kernel ở trạng thái nửa vời: hãy coi boot lỗi là không khởi động.
- Thứ tự `[objects]` ảnh hưởng địa chỉ vật lý (toml sắp xếp khoá), không ảnh hưởng ngữ nghĩa.
- Badge fault phải trùng slot TCB trong supervisor; đây là quy ước, công cụ build nên tự sinh để tránh sai.
