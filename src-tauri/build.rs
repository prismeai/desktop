fn main() {
    // ASWebAuthenticationSession lives in the AuthenticationServices framework.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=framework=AuthenticationServices");
    }
    tauri_build::build()
}
