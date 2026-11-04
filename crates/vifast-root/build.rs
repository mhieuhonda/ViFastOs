fn main() {
    for key in ["VIFAST_SERVER_ELF", "VIFAST_CLIENT_ELF"] {
        println!("cargo:rerun-if-env-changed={key}");
        if let Ok(path) = std::env::var(key) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}
