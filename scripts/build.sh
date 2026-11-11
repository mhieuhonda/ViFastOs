#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

BUILD_LOG="${VIFAST_BUILD_LOG:-target/vifast-build.log}"
mkdir -p "$(dirname "$BUILD_LOG")"
: > "$BUILD_LOG"
exec > >(tee "$BUILD_LOG") 2>&1

# r295 (bước 0): BUILD_ID — chặn đo nhầm binary cũ.
# Lý do: đã mất ít nhất một vòng vì đo một ELF build từ trạng thái nguồn cũ.
# Định dạng: <git sha ngắn>-<8 ký tự hash của git diff>. Phần đặc tả feature
# KHÔNG nằm trong id so sánh được (xem VIFAST_BUILD_FEATS) vì mỗi thành phần
# có thể được build với bộ feature khác nhau một cách hợp lệ (kernel/root bản
# `test-destroy-smp` đi với server/client bản thường) — nếu nhét vào id thì
# sẽ sinh T-BUILD GIẢ. Feature được in kèm ở trường `feats=` để vẫn chứng
# minh được "đang chạy đúng biến thể nào".
hash8() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | cut -c1-8
    else
        shasum -a 256 | cut -c1-8
    fi
}
BUILD_SHA="$(git rev-parse --short HEAD 2>/dev/null || printf 'nogit0')"
BUILD_DIFF="$(git diff HEAD 2>/dev/null | hash8 || printf '00000000')"
VIFAST_BUILD_ID="${BUILD_SHA}-${BUILD_DIFF}"
export VIFAST_BUILD_ID
export VIFAST_BUILD_FEATS="none"
echo "$VIFAST_BUILD_ID" > target/vifast-build-id.txt
echo "BUILD_ID=${VIFAST_BUILD_ID}"

echo "== host portable-kernel tests =="
cargo test -p vifast-kernel
cargo test -p vifast-abi
cargo test -p vifast-kernel --lib layout_sizes_for_status -- --nocapture
cargo test -p vifast-arch
cargo test -p vifast-pte
cargo test -p vifast-elf
# Vòng 166: test của FS (quy tắc đĩa, biên block, fsck) phải nằm trong cổng —
# đây là nơi dễ hỏng ngầm nhất khi sửa driver/định dạng đĩa.
cargo test -p vifast-fs
# Vòng 167: checksum/parse của net stack cũng phải nằm trong cổng.
cargo test -p vifast-virtio
# Vòng 175: bảng tra lệnh shell trong tài liệu phải khớp mã nguồn.
python3 scripts/gen-shell-table.py --check
# Vòng 176: bảng số test host cũng phải khớp kết quả thật.
python3 scripts/gen-test-counts.py --check
# Vòng 188: bảng marker trong runbook phải khớp harness_spec.
python3 scripts/gen-marker-table.py --check
cargo test -p vifast-runtime
cargo test -p vifast-user
cargo test -p vifast-acpi
cargo test -p vifast-fs
cargo test -p vifast-fsutil
cargo test -p vifast-virtio
cargo build -p vifast-fsutil
python3 scripts/test-harness-spec.py

echo "== root ELF build/validate (Step 6.3) =="
HOST="$(rustc -vV | sed -n 's/^host: //p')"
OBJCOPY="$(rustc --print sysroot)/lib/rustlib/${HOST}/bin/llvm-objcopy"
ROOT_RV=target/riscv64imac-unknown-none-elf/root-task/vifast-root
ROOT_ARM=target/aarch64-unknown-none-softfloat/root-task/vifast-root
ROOT_X86=target/x86_64-unknown-none/root-task/vifast-root
SERVER_RV=target/riscv64imac-unknown-none-elf/root-task/vifast-server
CLIENT_RV=target/riscv64imac-unknown-none-elf/root-task/vifast-client
SERVER_ARM=target/aarch64-unknown-none-softfloat/root-task/vifast-server
CLIENT_ARM=target/aarch64-unknown-none-softfloat/root-task/vifast-client
SERVER_X86=target/x86_64-unknown-none/root-task/vifast-server
CLIENT_X86=target/x86_64-unknown-none/root-task/vifast-client

build_modules() {
    target="$1"
    linker="$2"
    shift 2
    for pkg in vifast-server vifast-client; do
        # r249 (T13): client dùng script KHÔNG căn trang — mỗi khoảng đệm
        # 4 KiB là một trang ELF thật, và mỗi trang client tốn một Frame
        # staging lúc boot. Server giữ script cũ (nạp từ ảnh kernel, không
        # bị ngân sách trang client chi phối).
        case "$pkg" in
            vifast-client) module_linker=crates/vifast-root/linkers/module.ld ;;
            *) module_linker="$linker" ;;
        esac
        # r295 (bước 2): feature CHẨN ĐOÁN cho client (`stress-fs`), mặc định
        # KHÔNG bật — gate và bản phát hành dùng client thường.
        client_feats=""
        if [ "$pkg" = "vifast-client" ]; then
            client_feats="${VIFAST_CLIENT_FEATURES:-}"
        fi
        VIFAST_BUILD_FEATS="${client_feats:-none}" \
        cargo rustc -p "$pkg" --profile root-task --target "$target" \
            ${client_feats:+--features "$client_feats"} -- \
            "$@" \
            -C link-arg=-T -C link-arg="$module_linker" \
            -C link-arg=--defsym=ROOT_ELF_BASE=0x1000010000 \
            -C link-arg=-z -C link-arg=max-page-size=4096
    done
}

build_modules riscv64imac-unknown-none-elf crates/vifast-root/linkers/riscv64.ld \
    -C code-model=medium -C relocation-model=static
build_modules aarch64-unknown-none-softfloat crates/vifast-root/linkers/aarch64.ld \
    -C relocation-model=static
build_modules x86_64-unknown-none crates/vifast-root/linkers/x86_64.ld \
    -C relocation-model=pic -C link-arg=-no-pie

for elf in "$SERVER_RV" "$CLIENT_RV" "$SERVER_ARM" "$CLIENT_ARM" "$SERVER_X86" "$CLIENT_X86"; do
    "$OBJCOPY" --strip-all "$elf" "$elf.stripped"
done

# r237 (T5/D10): chương trình ngoài chạy từ FS.
# r247 (T12/D14): MỘT ảnh multicall `/bin/vf` cho mọi tên lệnh — mode chọn
# theo argv[0] qua ABI D15, nên không còn build 3 ảnh riêng.
build_bins() {
    target="$1"
    linker="$2"
    tag="$3"
    shift 3
    cargo rustc -p vifast-bin --profile root-task \
        --target "$target" -- "$@" \
        -C link-arg=-T -C link-arg=crates/vifast-root/linkers/module.ld \
        -C link-arg=--defsym=ROOT_ELF_BASE=0x1000010000 \
        -C link-arg=-z -C link-arg=max-page-size=4096 \
        -C link-arg=-z -C link-arg=separate-code
    out="target/$target/root-task/vifast-bin"
    "$OBJCOPY" --strip-all "$out" "$out.stripped"
    echo "bin(${tag}) $(wc -c < "$out.stripped" | tr -d ' ') B"
}

build_bins riscv64imac-unknown-none-elf crates/vifast-root/linkers/riscv64.ld rv \
    -C code-model=medium -C relocation-model=static
build_bins aarch64-unknown-none-softfloat crates/vifast-root/linkers/aarch64.ld arm \
    -C relocation-model=static
build_bins x86_64-unknown-none crates/vifast-root/linkers/x86_64.ld x86 \
    -C relocation-model=pic -C link-arg=-no-pie

bin_env() {
    tag="$1"
    export VIFAST_BIN_ELF="$PWD/target/$tag/root-task/vifast-bin.stripped"
}

bin_env riscv64imac-unknown-none-elf
VIFAST_BUILD_FEATS="${VIFAST_ROOT_FEATURES:-none}" cargo rustc -p vifast-root --profile root-task --target riscv64imac-unknown-none-elf ${VIFAST_ROOT_FEATURES:+--features "$VIFAST_ROOT_FEATURES"} -- \
    -C code-model=medium -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/riscv64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_RV" "$ROOT_RV.stripped"
scripts/check-root-elf.sh "$ROOT_RV.stripped" riscv64imac-unknown-none-elf 243
# Ngân sách trang cho client ELF: mỗi trang client bị staging cấp 1 Frame lúc
# boot, và Untyped dữ liệu chỉ còn ~6 trang. Vượt 6 trang sẽ làm demo PREEMPT
# OK timeout (bài học vòng 116) — chặn ngay ở build thay vì để QEMU đỏ khó hiểu.
# Ngân sách trang theo từng arch (đo từ bản đang xanh): riscv 6, aarch64 7,
# x86_64 8. Mỗi trang client bị staging cấp 1 Frame lúc boot; vượt ngân sách
# sẽ làm demo PREEMPT OK timeout như vòng 116.
check_client_budget() {
    local arch="$1" file="$2" limit="$3"
    if [ ! -f "$file" ]; then
        return 0
    fi
    local size pages
    size=$(wc -c < "$file" | tr -d ' ')
    pages=$(( (size + 4095) / 4096 ))
    echo "client(${arch}) ${size} B = ${pages} trang (ngân sách ${limit})"
    if (( pages > limit )); then
        echo "CLIENT ELF ${arch} VƯỢT NGÂN SÁCH TRANG (tối đa ${limit}): ${pages} trang" >&2
        echo "Thu gọn vifast-client hoặc nới ngân sách Untyped trước khi tăng." >&2
        exit 1
    fi
}
# Vòng 206: nâng 7 -> 8 SAU KHI đo arena: DATA UNTYPED pages=69/76 ⇒ còn 7
# trang; client +1 trang (70/76) vẫn đủ cho một lần exec (~5 trang) vì exec
# luôn destroy trước khi spawn lần kế. Đo lại sau khi nâng; nếu chạm 76 thì hạ.
check_client_budget riscv64 "${CLIENT_RV}.stripped" 8

bin_env aarch64-unknown-none-softfloat
VIFAST_BUILD_FEATS="${VIFAST_ROOT_FEATURES:-none}" cargo rustc -p vifast-root --profile root-task --target aarch64-unknown-none-softfloat ${VIFAST_ROOT_FEATURES:+--features "$VIFAST_ROOT_FEATURES"} -- \
    -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/aarch64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_ARM" "$ROOT_ARM.stripped"
scripts/check-root-elf.sh "$ROOT_ARM.stripped" aarch64-unknown-none-softfloat 183
check_client_budget aarch64 "${CLIENT_ARM}.stripped" 8

bin_env x86_64-unknown-none
VIFAST_BUILD_FEATS="${VIFAST_ROOT_FEATURES:-none}" cargo rustc -p vifast-root --profile root-task --target x86_64-unknown-none ${VIFAST_ROOT_FEATURES:+--features "$VIFAST_ROOT_FEATURES"} -- \
    -C relocation-model=pic \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/x86_64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code \
    -C link-arg=-no-pie
"$OBJCOPY" --strip-all "$ROOT_X86" "$ROOT_X86.stripped"
scripts/check-root-elf.sh "$ROOT_X86.stripped" x86_64-unknown-none 62
check_client_budget x86_64 "${CLIENT_X86}.stripped" 9

echo "== destroy-smp root variants =="
VIFAST_BUILD_FEATS=test-destroy-smp cargo rustc -p vifast-root --profile root-task --target riscv64imac-unknown-none-elf --features test-destroy-smp -- \
    -C code-model=medium -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/riscv64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_RV" "${ROOT_RV}.smp.stripped"
scripts/check-root-elf.sh "${ROOT_RV}.smp.stripped" riscv64imac-unknown-none-elf 243
bin_env riscv64imac-unknown-none-elf
VIFAST_BUILD_FEATS="${VIFAST_ROOT_FEATURES:-none}" cargo rustc -p vifast-root --profile root-task --target riscv64imac-unknown-none-elf ${VIFAST_ROOT_FEATURES:+--features "$VIFAST_ROOT_FEATURES"} -- \
    -C code-model=medium -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/riscv64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_RV" "$ROOT_RV.stripped"

VIFAST_BUILD_FEATS=test-destroy-smp cargo rustc -p vifast-root --profile root-task --target aarch64-unknown-none-softfloat --features test-destroy-smp -- \
    -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/aarch64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_ARM" "${ROOT_ARM}.smp.stripped"
scripts/check-root-elf.sh "${ROOT_ARM}.smp.stripped" aarch64-unknown-none-softfloat 183
bin_env aarch64-unknown-none-softfloat
VIFAST_BUILD_FEATS="${VIFAST_ROOT_FEATURES:-none}" cargo rustc -p vifast-root --profile root-task --target aarch64-unknown-none-softfloat ${VIFAST_ROOT_FEATURES:+--features "$VIFAST_ROOT_FEATURES"} -- \
    -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/aarch64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_ARM" "$ROOT_ARM.stripped"

VIFAST_BUILD_FEATS=test-destroy-smp cargo rustc -p vifast-root --profile root-task --target x86_64-unknown-none --features test-destroy-smp -- \
    -C relocation-model=pic \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/x86_64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code \
    -C link-arg=-no-pie
"$OBJCOPY" --strip-all "$ROOT_X86" "${ROOT_X86}.smp.stripped"
scripts/check-root-elf.sh "${ROOT_X86}.smp.stripped" x86_64-unknown-none 62
bin_env x86_64-unknown-none
VIFAST_BUILD_FEATS="${VIFAST_ROOT_FEATURES:-none}" cargo rustc -p vifast-root --profile root-task --target x86_64-unknown-none ${VIFAST_ROOT_FEATURES:+--features "$VIFAST_ROOT_FEATURES"} -- \
    -C relocation-model=pic \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/x86_64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code \
    -C link-arg=-no-pie
"$OBJCOPY" --strip-all "$ROOT_X86" "$ROOT_X86.stripped"

echo "== riscv64 bare metal =="
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release
NM="$(rustc --print sysroot)/lib/rustlib/${HOST}/bin/llvm-nm"
KERNEL_RV=target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64
# r254 (T17): bản kernel "shell linger" — TIMER_TICKS lớn để `--interactive`
# kịp gõ kịch bản. Gate và soak KHÔNG dùng bản này.
VIFAST_BUILD_FEATS=test-shell-linger VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release --features test-shell-linger
cp "$KERNEL_RV" target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-shell
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release
KERNEL_END="$("$NM" "$KERNEL_RV" | awk '$3 == "__kernel_end" { print "0x" $1; exit }')"
KERNEL_END=$((KERNEL_END))
KERNEL_LIMIT=$((0x80600000))
echo "RISC-V kernel_end=${KERNEL_END} limit=${KERNEL_LIMIT} headroom=$((KERNEL_LIMIT - KERNEL_END))"
if (( KERNEL_END > KERNEL_LIMIT )); then
    echo "RISC-V KERNEL IMAGE EXCEEDS 4 MiB" >&2
    exit 1
fi
if (( KERNEL_LIMIT - KERNEL_END < 196608 )); then
    echo "RISC-V KERNEL HEADROOM BELOW 192 KiB: $((KERNEL_LIMIT - KERNEL_END)) bytes" >&2
    exit 1
fi


echo "== riscv64 destroy-smp feature build =="
VIFAST_BUILD_FEATS=test-destroy-smp VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.smp.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release --features test-destroy-smp
KERNEL_VAR=target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-destroy-smp
cp "$KERNEL_RV" "$KERNEL_VAR"
KERNEL_END="$("$NM" "$KERNEL_VAR" | awk '$3 == "__kernel_end" { print "0x" $1; exit }')"
KERNEL_END=$((KERNEL_END))
echo "RISC-V destroy-smp kernel_end=${KERNEL_END} headroom=$((KERNEL_LIMIT - KERNEL_END))"
if (( KERNEL_LIMIT - KERNEL_END < 196608 )); then
    echo "RISC-V DESTROY-SMP HEADROOM BELOW 192 KiB" >&2
    exit 1
fi
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release

echo "== riscv64 boot-handoff test build =="
VIFAST_BUILD_FEATS=test-boot-handoff VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release --features test-boot-handoff
cp "$KERNEL_RV" target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-handoff
# Restore the normal artifact for all regular QEMU runs.
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release

echo "== riscv64 panic-halt feature build =="
VIFAST_BUILD_FEATS=test-panic-hart1 VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release --features test-panic-hart1
cp "$KERNEL_RV" target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-panic
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release
# TLB mutation build: expected to print TLB REMAP BAD and exit.
VIFAST_BUILD_FEATS=test-skip-tlb-ipi VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release --features test-skip-tlb-ipi
cp "$KERNEL_RV" target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-skip-tlb
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release

# r259 (T21/D19): biến thể kiểm bất biến sched ở RANH GIỚI trả về user.
# Bản `boundary` phải in BOUNDARY OK; bản `boundary-mut` cố ý làm hỏng bất
# biến đúng một lần (D18) nên phải in BOUNDARY BAD.
VIFAST_BUILD_FEATS=check-boundary VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release --features check-boundary
cp "$KERNEL_RV" target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-boundary
VIFAST_BUILD_FEATS=check-boundary-mutation VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release --features check-boundary-mutation
cp "$KERNEL_RV" target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-boundary-mut
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release

echo "== aarch64 bare metal =="
VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release
KERNEL_ARM=target/aarch64-unknown-none/release/vifast-kernel-aarch64
# r254 (T17): biến thể shell-linger cho aarch64 (xem chú thích ở riscv64).
VIFAST_BUILD_FEATS=test-shell-linger VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release --features test-shell-linger
cp "$KERNEL_ARM" target/aarch64-unknown-none/release/vifast-kernel-aarch64-shell
VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release
# KERNEL_STORAGE is a ~560 KiB static in .data placed just before BKL.
# If Kernel grows, reset() silently clobbers BKL/KERNEL_PTR (vòng 30 DABT).
# Fail loudly here by measuring the real static size from the binary.
KS_INFO="$("$NM" --print-size "$KERNEL_ARM" | awk '$4 ~ /KERNEL_STORAGE/ { print "0x" $1 " 0x" $2; exit }')"
BKL_ADDR="$("$NM" "$KERNEL_ARM" | awk '$3 ~ /platform3BKL$/ { print "0x" $1; exit }')"
KS_ADDR=$(( ${KS_INFO%% *} ))
KS_SIZE=$(( ${KS_INFO##* } ))
BKL_ADDR=$((BKL_ADDR))
# r229 (T4): `KERNEL_STORAGE` là struct gồm Kernel + vùng đệm 4 KiB, nên khe
# thật giữa Kernel và static kế tiếp = BKL - (storage + size - guard).
KS_GUARD=4096
GAP=$((BKL_ADDR - (KS_ADDR + KS_SIZE - KS_GUARD)))
echo "AArch64 static check storage=${KS_ADDR}+${KS_SIZE} (Kernel=$((KS_SIZE - KS_GUARD)) + guard=${KS_GUARD}) bkl=${BKL_ADDR} gap=${GAP}"
if (( KS_SIZE < KS_GUARD + 1024 )); then
    echo "AArch64 KERNEL STORAGE THIẾU GUARD: size=${KS_SIZE} < guard+1K" >&2
    exit 1
fi
if (( GAP < 4096 )); then
    echo "AArch64 KERNEL STATIC GAP FAIL: chỉ ${GAP} byte trước BKL (yêu cầu >= 4096)." >&2
    echo "Bảo vệ: vùng đệm .data.kernel_storage_guard phải nằm ngay sau KERNEL_STORAGE." >&2
    exit 1
fi
if (( KS_ADDR + KS_SIZE >= BKL_ADDR )); then
    echo "AArch64 KERNEL STATIC OVERFLOW: storage end past BKL" >&2
    exit 1
fi

echo "== aarch64 destroy-smp feature build =="
VIFAST_BUILD_FEATS=test-destroy-smp VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.smp.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release --features test-destroy-smp
cp target/aarch64-unknown-none/release/vifast-kernel-aarch64 target/aarch64-unknown-none/release/vifast-kernel-aarch64-destroy-smp
VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release

echo "== aarch64 panic-halt feature build =="
VIFAST_BUILD_FEATS=test-panic-hart1 VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release --features test-panic-hart1
cp target/aarch64-unknown-none/release/vifast-kernel-aarch64 target/aarch64-unknown-none/release/vifast-kernel-aarch64-panic
VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release

echo "== x86_64 UEFI =="
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release

echo "== x86_64 destroy-smp feature build =="
VIFAST_BUILD_FEATS=test-destroy-smp VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.smp.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release --features test-destroy-smp
cp target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi.efi target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi-destroy-smp.efi
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release

echo "== x86_64 panic-halt feature build =="
VIFAST_BUILD_FEATS=test-panic-hart1 VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release --features test-panic-hart1
cp target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi.efi target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi-panic.efi
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release
VIFAST_BUILD_FEATS=test-skip-tlb-ipi VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release --features test-skip-tlb-ipi
cp target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi.efi target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi-skip-tlb.efi
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release

# r259 (T21/D19): biến thể kiểm bất biến sched ở ranh giới user (x86_64).
VIFAST_BUILD_FEATS=check-boundary VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release --features check-boundary
cp target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi.efi target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi-boundary.efi
VIFAST_BUILD_FEATS=check-boundary-mutation VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release --features check-boundary-mutation
cp target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi.efi target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi-boundary-mut.efi
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release

echo "build complete"
