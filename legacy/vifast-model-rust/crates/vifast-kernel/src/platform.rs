//! Giao diện giữa logic kernel và phần cứng.
//!
//! Trên phần cứng thật, `vifast-core` cài đặt trait này bằng `unsafe`:
//! đọc/ghi bộ nhớ vật lý qua vùng direct map, `sfence.vma` + IPI qua SBI, UART.
//! Trên host, `vifast-sim` cài đặt bằng một mảng byte và TLB giả lập.

pub trait Platform {
    /// Đọc bộ nhớ vật lý (chỉ RAM). Trả `false` nếu địa chỉ không phải RAM.
    fn phys_read(&self, pa: u64, buf: &mut [u8]) -> bool;

    /// Ghi bộ nhớ vật lý (chỉ RAM). Trả `false` nếu địa chỉ không phải RAM.
    fn phys_write(&mut self, pa: u64, data: &[u8]) -> bool;

    /// Xoá TLB của một hart. `asid = None`: mọi ASID; `vpn = None`: mọi trang.
    /// Phần cứng thật: `sfence.vma` tại chỗ, hoặc IPI (SBI remote fence) cho hart khác.
    fn tlb_flush(&mut self, hart: usize, asid: Option<u32>, vpn: Option<u64>);

    /// Ghi log của kernel (UART trên phần cứng thật).
    fn log(&mut self, tick: u64, source: &str, text: &str);
}
