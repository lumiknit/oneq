use std::{env, fs, path::Path};

fn compress_asset(out_dir: &Path, src: &str, dest_name: &str) {
    println!("cargo:rerun-if-changed={src}");

    let data = fs::read(src).unwrap_or_else(|e| panic!("failed to read {src}: {e}"));
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&data, 10);

    let dest = out_dir.join(dest_name);
    fs::write(&dest, compressed)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", dest.display()));
}

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let out_dir = Path::new(&out_dir);

    println!(
        "cargo:rustc-env=BUILD_TARGET={}",
        std::env::var("TARGET").unwrap()
    );

    let output = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .unwrap();
    let version = String::from_utf8(output.stdout).unwrap();
    println!("cargo:rustc-env=RUSTC_VERSION={}", version.trim());

    let opt_level = std::env::var("OPT_LEVEL").unwrap();
    let debug = std::env::var("DEBUG").unwrap();
    let profile = std::env::var("PROFILE").unwrap();

    println!("cargo:rustc-env=BUILD_OPT_LEVEL={opt_level}");
    println!("cargo:rustc-env=BUILD_DEBUG={debug}");
    println!("cargo:rustc-env=BUILD_PROFILE={profile}");

    compress_asset(out_dir, "src/doc/doc.json", "doc.json.zz");
}
