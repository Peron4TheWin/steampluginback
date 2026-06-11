fn main() {
    println!("cargo:rustc-link-arg=/FORCE:MULTIPLE");
    println!("cargo:rustc-link-arg=/DEF:wsock32.def");
}
