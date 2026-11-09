# ViFastOs brand assets

Logo chính thức của dự án. Tên hiển thị luôn viết **ViFastOs** (PascalCase).

## File

| File | Dùng cho |
|---|---|
| `vifastos-icon.svg` | icon gốc (nguồn thiết kế), nền trong |
| `vifastos-icon-512.png` | app icon, avatar, manifest |
| `vifastos-icon-192.png` | manifest / PWA / launcher |
| `vifastos-lockup.svg` | logo ngang gốc: icon + wordmark + tagline `MICROKERNEL RUST · CAPABILITY` |
| `vifastos-lockup.png` | logo ngang nền trong suốt (cho nền tối) |
| `vifastos-lockup-cream.png` | README, web, tài liệu — nền kem `#FFF8EC`, 1400×400 |

`vifastos-lockup-cream.png` được dựng từ `vifastos-lockup.svg` (render 2× rồi hạ về
1400×400) để font không bị lệch giữa macOS / Windows / Linux. Bản dùng trên web nằm ở
`deploy/vifastos_web/static/brand/` (icon SVG/PNG, favicon, `og-1200.png`).

## Màu

| Vai trò | Màu |
|---|---|
| Đỏ thương hiệu (tile, chữ "Vi" và "Os") | `#E8352B` → `#B3150F` (gradient) |
| Sao vàng | `#FFE04A` → `#FFB800` |
| Vạch tốc độ | `#FFD84D` |
| Nền kem | `#FFF8EC` |
| Chữ "Fast" (nghiêng) | `#1F2430` |
| Tagline | `#6B7280` |

## Quy tắc

- Icon là **viên đỏ bo góc** `rx=112` trên lưới 512, có **lục giác 2 tầng** (viền vàng
  ngoài, viền kem mờ bên trong), **ngôi sao 5 cánh** và **ba vạch tốc độ** (vạch giữa
  thụt vào, mép phải thẳng hàng).
- Wordmark luôn là `Vi` đỏ + `Fast` nghiêng + `Os` đỏ; không đổi thứ tự, không viết
  `VIFASTOS`, `ViFast OS` hay `vifastos`.
- Không dùng icon trên nền đỏ cùng tông, không bóp méo tỉ lệ, chừa lề ≥ 12% cạnh ngắn.

Giấy phép: MIT hoặc Apache-2.0 (xem `LICENSE-MIT` / `LICENSE-APACHE` ở gốc repo). Chi tiết
nhãn hiệu: `TRADEMARKS.md`.
