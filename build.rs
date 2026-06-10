fn main() {
    // ws2_32.lib define las mismas funciones que exportamos (accept, bind, send, etc.)
    // causando LNK2005. Lo excluimos porque nosotros somos el wsock32 proxy.
    println!("cargo:rustc-link-arg=/NODEFAULTLIB:ws2_32.lib");
}