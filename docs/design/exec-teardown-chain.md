# Chuỗi thu hồi sau exec (exec-teardown-chain)

Trạng thái: **implemented** — chốt ở vòng 199–205, tài liệu hóa r234.
Bằng chứng: `destroy_cap_object` + `vspace_in_use` (`crates/vifast-kernel`),
test hồi quy `exec_teardown_chain_frees_tcb_vspace_frame` (vòng 205),
marker `EXEC MEM OK` / `EXEC MEM STABLE`.

## 1. Vấn đề

Mỗi lần `runfs` (exec-from-FS) tạo ra một chuỗi đối tượng mới:

```text
Untyped ──retype──> VSpace ──map──> PageTable(s) ──> Frame(s)
        └─retype──> Tcb
```

Khi chương trình kết thúc, chuỗi này phải bị thu hồi **đủ**. Trước vòng 192,
mỗi chu kỳ exec rò **6 object**: sau ~20 chu kỳ thì hết slot, `runfs` thứ N
báo lỗi khó hiểu và demo sau chết.

## 2. Ba nguyên nhân gốc (tìm ra ở vòng 192, 194, 199)

| # | Nguyên nhân | Vòng | Sửa |
|---|---|---|---|
| 1 | Slot bị mất **trong chính hàm spawn** — mảng slot ghi vào bản sao khác (`SPAWN LEN=6`) | 192 | truyền đúng mảng + ghi lại index |
| 2 | `Tcb` không bao giờ được hủy vì luôn ở trạng thái `Busy` | 194 | sửa điều kiện Busy trước khi destroy |
| 3 | `destroy_cap_object` **không cho phép** `ObjType::Tcb` | 199 | thêm `Tcb` vào nhánh được phép |
| 4 | `vspace_in_use` chặn **chính thứ nó sắp dọn** (tự deadlock điều kiện) | 200 | kiểm "in use" trước khi bắt đầu dọn, không kiểm giữa chừng |

Điểm thứ 4 là bài học quan trọng nhất: điều kiện "còn dùng" phải được đánh giá
**một lần ở đầu** thao tác, không phải trong lúc đang tháo dỡ — nếu không, thao
tác tự chặn mình.

## 3. Thứ tự thu hồi (bất biến)

```text
1. Tcb        (dừng chạy, gỡ khỏi runqueue + endpoint/notification)
2. VSpace     (unmap toàn bộ range, flush TLB theo mask harts_loaded)
3. PageTable  (trả page bảng sau khi cha không còn entry)
4. Frame      (trả về Untyped đã cấp)
```

Bất biến:

- **E1.** Không Frame nào bị trả trước khi PageTable trỏ tới nó bị gỡ.
- **E2.** Không VSpace nào bị destroy khi còn hart trong `harts_loaded`.
- **E3.** Mỗi bước là **idempotent**: gọi lại trên đối tượng đã chết trả
  `ObjectDestroyed`, không panic.
- **E4.** Tổng frame/TCB trả về phải bằng tổng đã cấp cho chu kỳ đó — kiểm bằng
  đếm sau mỗi lần exec, và bằng `EXEC MEM STABLE` sau nhiều vòng.

## 4. Test bảo vệ

| Test | Nội dung |
|---|---|
| `exec_teardown_chain_frees_tcb_vspace_frame` (v205) | chạy một chu kỳ exec rồi khẳng định TCB + VSpace + frame đều về Untyped |
| `exec_mem_stable` (marker `EXEC MEM STABLE`) | nhiều chu kỳ exec, đếm frame/TCB không đổi |
| `frame/PTE + ObjectDestroy` host test (v37x) | destroy VSpace khi còn hart → từ chối; sau khi xoá mask → cho phép |
| đo rò rỉ (v186/187) | `free` sau 7 lần `runfs`: 16 → 36 KiB, `DATA LEAKED 0` |

## 5. Bài học

1. **Kết luận "không rò rỉ" từ một lần đo là sai** (v187 kết luận sai; phải tới
   v192–v200 mới ra 3 root cause). Từ nay: đo lặp ≥ 3 chu kỳ hoặc mutation (D13).
2. **Ba root cause chồng nhau** — sửa một cái vẫn rò, nên phải đo lại sau mỗi
   lần sửa thay vì tin "đã xong".
3. **Chẩn đoán tự gây lỗi**: hai vòng 195–197 thêm `klog` trong đường nóng làm
   cổng đỏ và làm chính chẩn đoán không bao giờ in ra (đi nhánh khác). Chẩn đoán
   phải nằm ngoài đường quyết định.
