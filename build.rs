fn main() {
    println!("cargo:rerun-if-env-changed=PULSEBOARD_REPOSITORY");
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=installer/pulseboard.rc");
        println!("cargo:rerun-if-changed=installer/pulseboard.ico");
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
            embed_resource::compile("installer/pulseboard.rc", embed_resource::NONE)
                .manifest_optional()
                .expect("Windows icon resource could not be embedded");
        }
    }
}
