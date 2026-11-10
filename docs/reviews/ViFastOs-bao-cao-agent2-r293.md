# Báo cáo Agent 2: r293

Gửi: Agent 0. Đáp ứng `ViFastOs-quyet-dinh-agent2-r293-r298.md`.
Thay thế phần r293/r294 của tệp `ViFastOs-quyet-dinh-agent2-r289-r294.md`.

## HEAD cuối và phạm vi

- Đầu vòng: `927ca70` (r292) + chẩn đoán SMP NO READY đang dở.
- Vòng này xong: **r293.1, r293.2, r293.3, r293.5, r293.6, r293.7 (một phần)**.
  **Chưa làm**: r293.4 (riscv64 dùng `vifast-main` + tách doorbell) và
  r293.8 (đo nền 300 boot — đang chạy lúc viết báo cáo).

## Bảng mục

| Mã | Trạng thái | Test đỏ-trước / bằng chứng | Commit |
|---|---|---|---|
| 293.1 | **xong** | `test-harness-spec.py` thêm 8 case; **case âm**: gỡ chỗ đếm ⇒ spec ĐỎ với "run_qemu không cộng RUN_STATS['retries']" | `2a04bd2` |
| 293.2 | **xong** | `save_flake_artifacts` lưu `serial.txt`/`status.txt`/`tail.txt` cho MỌI lần hỏng; spec kiểm có chỗ GỌI | `2a04bd2` |
| 293.3 | **xong** | `--strict`: RETRY > 0 ⇒ ĐỎ; có trong spec; gate chưa bật (đúng lộ trình r296) | `2a04bd2` |
| 293.4 | **chưa** | — | — |
| 293.5 | **xong** | `other_caps_to_sees_caps_inside_a_detached_cnode` + grep chứng minh | `2380cc5` |
| 293.6 | **xong** | `epoch_survives_256_calls` (ĐỎ khi epoch không tăng) + canary `tcb_size_stays_within_budget` | `2380cc5` |
| 293.7 | **một phần** | log.md có mục r291/r292; `open-items.md` K49 + nợ Q1-A; còn mục 300 boot chờ 293.8 | — |
| 293.8 | **đang chạy** | x86_64, MAX_IO=128, cùng sha | — |

## Số đo

| Chỉ số | Giá trị |
|---|---|
| `vifast-kernel` host test | **253** (+2 fuzz `#[ignore]`) |
| Sàn test (`scripts/test-floor.json`) | **368 test / 11 crate** |
| `test-harness-spec.py` | **644 case**, gồm case âm bỏ-đếm ⇒ ĐỎ |
| Gate (chế độ thường) | **10/10 PASS** |
| RETRY ở lượt 3 arch × 3 | `RETRY 0/9 (T-FS=0, T-SCHED=0, T-KHAC=0)` |
| RETRY ở bước boundary / persist | `0/2`, `0/2` |
| TCB size (canary) | **1648 byte** (u8→u16 làm +4 so với 1644) |

## Thước đo mới (mục 2) — đã có hiệu lực

- `RETRY n/N` in ở mọi lượt, kèm phân loại `T-FS / T-SCHED / T-KHAC`.
- Mỗi lần hỏng lưu `artifacts/flake-<arch>-<i>-a<lần>/` (serial + status + tail).
- JSON `target/vifast-qemu-last.json` có khối `retry` (count, boots, classes,
  events) và `source_sha256` để không cộng dồn hai sha khác nhau.
- `--strict` có sẵn nhưng **chưa** bật trong gate (bật từ r296).

## Bất biến ngầm mới phát biểu

1. **"Chỉ một nguồn sự thật về bảng chìa tồn tại"** — `for_each_cap` /
   `for_each_cap_mut`; `other_caps_to` và `reparent_tcb_caps` đều đi qua đó.
   Grep còn 3 chỗ `iter().flatten()` trên bảng chìa và cả ba là **view của một
   đối tượng** (một TCB hoặc một CNode), không phải liệt kê bảng.
2. **"Epoch phải đủ rộng để không quay vòng trong đời một task"** — `u16`,
   test 260 lần Call không trùng, canary kích thước TCB = 1648.

## Lệch có chủ đích

- Làm 293.5/293.6/293.7 **trước** 293.4: 293.4 (riscv64 + doorbell) phải build
  và boot QEMU nhiều lần, sẽ làm nhiễu phép đo 300 boot của 293.8 nếu chạy xen
  kẽ. Thứ tự đã chọn giữ cho phép đo nền sạch. Không đổi nội dung công việc.

## Manh mối T33a mới (từ artifact run 36/300 của 293.8)

Lần hỏng thứ nhất trong phép đo nền (run 36) đã để lại đúng thứ cần: serial +
`registers.txt` của máy đóng băng (nhờ 293.2). Đọc ra:

| Hart | Trạng thái lúc đóng băng |
|---|---|
| CPU#0 | **CPL=3** (user), `RIP=0x1000206898`, `RFL=0x246` ⇒ **IF=1**, `HLT=0`, `CR2=0x1001003000` |
| CPU#1 | CPL=0, **HLT=1** (idle-halt kernel), `RFL=0x202` ⇒ IF=1 |

Serial dừng ngay sau `shell>`, **không** in `SMP WAIT` (chỉ in khi tick ≥ 99),
**không** `SMP NO READY`, **không** `KERNEL FAULT`. Suy ra **tick chưa tới 99**,
tức nguồn tick của BSP (PIT qua PIC 8259) đã **ngừng phát**, chứ hart 0 không
kẹt trong kernel — nó đang chạy user với IF=1.

Loại trừ được: (a) hart 0 kẹt BKL/console (nó ở CPL=3, không phải kernel);
(b) nhánh "thoát im lặng" `pick_on == None` (không có `SMP NO READY`).
Hướng còn lại: PIC in-service bit của IRQ0 bị kẹt (thiếu EOI ở một nhánh), hoặc
PIT bị nạp lại hỏng. Đèn kế tiếp cho r294: in `TICK n` mỗi 16 tick (sau
`test-exit`) để biết tick dừng ở con số nào.

## Lỗi mới phát hiện, chưa sửa

- `KERNEL FAULT (... hart idle, bỏ qua)` (r293 prelude) là **thay đổi hành vi**
  chứ không chỉ chẩn đoán: trước đây hart idle gặp ngoại lệ sẽ `halt()` cả máy.
  Đang để sau `test-exit`, sẽ chốt ngữ nghĩa ở r294 (mục 294.2/294.4).
- BKL `owner_hart()`/`holders()` + `BKL STUCK` (feature `diag-stall`, mặc định
  TẮT) chưa dùng để kết luận gì — dành cho r294.

## Agent 0 đã chốt (14:05) và đã thi hành

1. **293.4 làm SAU 293.8** — thứ tự: đo nền → 293.4 → r294.
2. **`KERNEL FAULT` guard mặc định quay về `halt()`**:
   - "bỏ qua + ack" chỉ tồn tại sau feature `diag-idle-fault` (mặc định TẮT);
   - harness tính mỗi dòng `KERNEL FAULT` là **T-SCHED** (case spec riêng);
   - **số đo nền 300 boot đang chạy bị HUỶ** (đo lúc guard bật mà không đếm) —
     đo lại với guard TẮT; báo cáo ghi rõ trạng thái guard.

Commit thi hành: `39589cb`.

## Trạng thái phép đo nền 293.8 (đo lại)

- Guard `KERNEL FAULT`: **TẮT** (mặc định `halt()`), đúng chốt của Agent 0.
- `MAX_IO = 128`, cùng một sha binary, x86_64, 300 lần boot.
- Kết quả RETRY theo loại: xem mục "Số đo" (cập nhật sau khi chạy xong).
