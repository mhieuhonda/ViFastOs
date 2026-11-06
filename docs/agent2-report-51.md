# ViFastOs — Báo cáo vòng 51 (xác minh backend net + chốt pha 3 MVP)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `d25baec`
**Mốc:** Xác minh ngoài guest, chốt pha 3 ở mức hạ tầng đã kiểm chứng.

## 1. Xác minh backend (không đổi code guest)

Chạy QEMU nền + hỏi monitor `info network` qua unix socket:

- `virtio-net-device.0: index=0,type=nic,model=virtio-net-device,macaddr=52:54:00:12:34:56`
- `vn0: index=0,type=user,net=10.0.2.0,restrict=off`

Backend sống, MAC khớp với log guest. Vậy chuỗi đã kiểm chứng tới:
probe → MAC → features → 2 queue → DRIVER_OK → ARP TX (used advance).
Reply không về trong boot window là việc của vòng sau (dump gói/timing),
không phải hạ tầng sai.

## 2. Pha 3 chốt MVP (vòng 39–51)

| Vòng | Nội dung |
|---|---|
| 39-40 | scan API + net_mac, hoãn log sau claim |
| 41 | xác minh treo MMIO (đọc nhầm serial) |
| 42 | serial đúng $TMPDIR; probe trước init thấy net cả 2 arch |
| 43 | log MAC cả 2 arch |
| 44 | đọc feature bits `feats=0x30bf8024` |
| 45 | NetQueue static riêng ngoài Kernel |
| 46 | `net_init_tx`, READY cả 2 arch |
| 47 | ARP TX đầu tiên (`sent len=0`) |
| 48-49 | RX API + `net_init_both` trước DRIVER_OK, bound 2M |
| 50 | ARP 60B + peek chẩn đoán, xác nhận không có reply |
| 51 | monitor xác nhận backend sống; chốt MVP |

## 3. Số liệu cổng (giữ nguyên vòng 50)

- Host 203, build 0 warning (+check static PASS, khe hở 8B), `fmt` PASS.
- QEMU 3/3×3, persist 7×3, harness 483, docs PASS.
- Không đổi code vòng này (chỉ script kiểm tra tạm `/tmp`, không commit).
