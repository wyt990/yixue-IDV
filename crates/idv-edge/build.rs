use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_windres() -> Option<PathBuf> {
    if let Some(p) = env::var_os("WINDIRES") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(path) = env::var("PATH") {
        for dir in path.split(';') {
            let p = Path::new(dir).join("windres.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    for p in [
        r"C:\msys64\mingw64\bin\windres.exe",
        r"C:\msys64\ucrt64\bin\windres.exe",
        r"C:\msys64\clang64\bin\windres.exe",
    ] {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn main() {
    if !cfg!(windows) {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let icon = manifest.join("../../assets/icons/idv-edge.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    if !icon.is_file() {
        println!("cargo:warning=icon not found: {}", icon.display());
        return;
    }
    let Some(windres) = find_windres() else {
        println!("cargo:warning=windres.exe not found; exe icon skipped");
        return;
    };
    let out = PathBuf::from(env::var("OUT_DIR").expect("out dir"));
    let rc = out.join("idv-edge-icon.rc");
    let obj = out.join("idv-edge-icon.o");
    let icon_path = icon.to_string_lossy().replace('\\', "/");
    if std::fs::write(&rc, format!("IDI_ICON1 ICON \"{icon_path}\"\n")).is_err() {
        println!("cargo:warning=failed to write resource script");
        return;
    }
    let status = Command::new(&windres)
        .arg("-i")
        .arg(&rc)
        .arg("-o")
        .arg(&obj)
        .arg("-O")
        .arg("coff")
        .status();
    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg={}", obj.display());
        }
        _ => println!("cargo:warning=windres failed; exe icon skipped"),
    }
}
