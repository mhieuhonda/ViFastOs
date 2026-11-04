//! Chạy thử: `cargo run -p vifast-sim -- space` hoặc `-- desktop`.

use vifast_sim::{build_str, default_images, DESKTOP_MANIFEST, SPACE_MANIFEST};

fn main() {
    let profile = std::env::args().nth(1).unwrap_or_else(|| "space".to_string());
    let manifest = match profile.as_str() {
        "space" => SPACE_MANIFEST,
        "desktop" => DESKTOP_MANIFEST,
        other => {
            eprintln!("profile không hợp lệ: {} (dùng space hoặc desktop)", other);
            std::process::exit(2);
        }
    };
    let mut m = match build_str(manifest, default_images(), true) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("boot lỗi: {}", e);
            std::process::exit(1);
        }
    };
    if profile == "space" {
        m.schedule_irq(5, 2, 8); // cảm biến IMU phát ngắt mỗi 8 tick
        m.run(1500);
    } else {
        m.run(2000);
    }
    println!("\nThống kê:");
    for (k, v) in m.report() {
        println!("  {:<16} {}", k, v);
    }
    let canary = if m.kernel_canary_intact() { "nguyên vẹn" } else { "BỊ GHI ĐÈ" };
    println!("  {:<16} {}", "kernel canary", canary);
}
