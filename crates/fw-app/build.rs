//! Embeds the icon and version info into the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/fretwright.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/fretwright.ico");
        res.set("ProductName", "FretWright");
        res.set("FileDescription", "FretWright: audio to Rocksmith 2014 bass arrangements");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
