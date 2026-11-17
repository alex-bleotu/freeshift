use std::env;
use std::fs;
use std::path::PathBuf;

fn skip_mingw_default_manifest() {
    let is_windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let is_gnu = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu");
    if !is_windows || !is_gnu {
        return;
    }
    let output = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("no-default-manifest");
    fs::create_dir_all(&output).expect("create no-default-manifest directory");
    let mut empty_object = [0u8; 20];
    empty_object[0] = 0x64;
    empty_object[1] = 0x86;
    fs::write(output.join("default-manifest.o"), empty_object).expect("write empty default-manifest.o");
    println!("cargo:rustc-link-arg=-B{}/", output.display());
}

fn main() {
    skip_mingw_default_manifest();
    tauri_build::build()
}
