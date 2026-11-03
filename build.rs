#[cfg(windows)]
fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    let mut resource = winres::WindowsResource::new();
    resource.set_icon("assets/app.ico");
    resource.set("FileDescription", "Kiseki Autorun Vault - キセキのオートラン書庫");
    resource.set("ProductName", "Kiseki Autorun Vault");
    resource.set("ProductVersion", "1.3.0");
    resource.compile().expect("Windows icon resource compilation failed (Windows SDK / rc.exe required)");
}

#[cfg(not(windows))]
fn main() {}
