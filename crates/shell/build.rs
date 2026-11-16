use std::path::PathBuf;

fn main() {
    slint_build::compile("ui/app.slint").expect("Slint UI failed to compile");

    // Discord Social SDK (downloaded separately into third_party/, not in git): link it when present.
    println!("cargo:rustc-check-cfg=cfg(discord_sdk)");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let sdk = manifest.join("../../third_party/discord_social_sdk");
    let lib = sdk.join("lib/release/discord_partner_sdk.lib");
    let dll = sdk.join("bin/release/discord_partner_sdk.dll");
    println!("cargo:rerun-if-changed={}", lib.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && lib.is_file() && dll.is_file() {
        println!("cargo:rustc-link-search=native={}", lib.parent().unwrap().display());
        println!("cargo:rustc-cfg=discord_sdk");
        // Put the DLL next to Win2Console.exe: the OUT_DIR ancestor named after the profile (debug/release).
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let profile = std::env::var("PROFILE").unwrap_or_default();
        if let Some(profile_dir) = out.ancestors().find(|p| p.file_name().is_some_and(|n| n == profile.as_str())) {
            if let Err(e) = std::fs::copy(&dll, profile_dir.join("discord_partner_sdk.dll")) {
                println!("cargo:warning=couldn't copy the Discord SDK DLL: {e}");
            }
        }
    }
}
