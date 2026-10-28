// Windows: when vendor/ViGEmBus_1.22.0_x64_x86_arm64.exe is there (vendor/fetch.ps1 gets it), the
// gamepad driver installer is built into the exe, so the PC doesn't need GitHub to install it.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(vigem_bundled)");
    println!("cargo::rerun-if-changed=vendor/ViGEmBus_1.22.0_x64_x86_arm64.exe");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    if windows && std::path::Path::new("vendor/ViGEmBus_1.22.0_x64_x86_arm64.exe").exists() {
        println!("cargo::rustc-cfg=vigem_bundled");
    }
}
