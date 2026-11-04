fn main() {
    println!("cargo:rerun-if-env-changed=VIFAST_ROOT_ELF");
    if let Ok(path) = std::env::var("VIFAST_ROOT_ELF") {
        println!("cargo:rerun-if-changed={path}");
    }
}
