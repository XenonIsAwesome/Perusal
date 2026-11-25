// Windows: embed the app icon and version info into perusal.exe (taskbar, Explorer, installers).
fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/perusal.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/perusal.ico");
        res.set("ProductName", "Perusal").set("FileDescription", "Perusal").set("CompanyName", "Perusal contributors");
        if let Err(e) = res.compile() { println!("cargo:warning=could not embed the Windows icon: {e}"); }
    }
}
