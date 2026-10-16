fn main() {
    slint_build::compile("ui/app.slint").expect("Slint build failed");

    // On Windows, embed the application icon into the .exe so it shows in
    // Explorer, the taskbar, and the title bar.
    #[cfg(windows)]
    {
        let ico = "resources/icons/app_icon.ico";
        if std::path::Path::new(ico).exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(ico);
            if let Err(e) = res.compile() {
                // Don't fail the whole build if the resource compiler is
                // unavailable; just warn.
                println!("cargo:warning=failed to embed Windows icon: {e}");
            }
        } else {
            println!("cargo:warning=resources/icons/app_icon.ico not found; skipping icon embed");
        }
        println!("cargo:rerun-if-changed={ico}");
    }
}
