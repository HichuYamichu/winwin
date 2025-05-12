use std::env;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let search_path = out_dir
        .ancestors()
        .nth(3)
        .expect("Failed to find target/debug directory");

    println!("cargo:rustc-link-search=native={}", search_path.display());
    println!("cargo:rustc-link-lib=dylib=hooks.dll");
}
