fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-arg=-T{}/linker.ld", dir);
    println!("cargo:rustc-link-arg=--gc-sections");
    println!("cargo:rerun-if-changed=linker.ld");
    println!("cargo:rerun-if-changed=src/boot.S");
    println!("cargo:rerun-if-env-changed=VIFAST_ROOT_ELF");
    if let Ok(path) = std::env::var("VIFAST_ROOT_ELF") {
        println!("cargo:rerun-if-changed={path}");
    }
}
