"""Chạy thử: python run_demo.py space | desktop"""
import os
import sys

from vifast_kernel import IMAGES, build, load_manifest

HERE = os.path.dirname(os.path.abspath(__file__))


def main() -> None:
    profile = sys.argv[1] if len(sys.argv) > 1 else "space"
    k = build(load_manifest(os.path.join(HERE, "manifests", f"{profile}.toml")), IMAGES, verbose=True)
    if profile == "space":
        k.schedule_irq(5, at=2, period=8)     # cảm biến IMU phát ngắt mỗi 8 tick
        k.run(max_ticks=1500)
    else:
        k.run(max_ticks=2000)
    print("\nThống kê:")
    for key, value in sorted(k.report().items()):
        print(f"  {key:<16} {value}")
    print(f"  kernel canary    {'nguyên vẹn' if k.phys.kernel_canary_intact() else 'BỊ GHI ĐÈ'}")


if __name__ == "__main__":
    main()
