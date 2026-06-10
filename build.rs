fn main() {
    println!("cargo:rustc-link-arg-cdylib=/NODEFAULTLIB:ws2_32.lib");
}