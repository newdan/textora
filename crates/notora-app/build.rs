use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let assets =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets the crate path"))
            .join("../../assets");
    let resource_script = assets.join("NotoraAppIcon.rc");
    let icon = assets.join("NotoraAppIcon.ico");
    let compiled_resource =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets the build output path"))
            .join("NotoraAppIcon.res");

    println!("cargo:rerun-if-changed={}", resource_script.display());
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-env-changed=RC");

    let target = env::var("TARGET").expect("Cargo sets the target triple");
    let mut resource_compiler = if let Some(override_path) = env::var_os("RC") {
        Command::new(override_path)
    } else {
        cc::windows_registry::find(&target, "rc.exe").unwrap_or_else(|| Command::new("rc.exe"))
    };
    let status = resource_compiler
        .current_dir(&assets)
        .arg("/nologo")
        .arg(format!("/fo{}", compiled_resource.display()))
        .arg(&resource_script)
        .status()
        .expect("Windows resource compiler must be available to embed the notora app icon");
    assert!(status.success(), "Windows resource compiler failed to embed the notora app icon");

    println!("cargo:rustc-link-arg-bin=notora={}", compiled_resource.display());
}
