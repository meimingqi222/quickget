fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(target_os = "macos")]
    {
        println!("cargo:rerun-if-changed=assets/icon.icns");
        println!("cargo:rerun-if-changed=assets/icon-512.png");
        println!("cargo:rerun-if-changed=assets/icon.png");
    }

    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/icon.ico");

        let mut res = winres::WindowsResource::new();
        if std::path::Path::new("assets/icon.ico").exists() {
            res.set_icon("assets/icon.ico");
        }
        res.set("FileDescription", "QuickGet 高速多协议下载器");
        res.set("ProductName", "QuickGet");
        res.set("OriginalFilename", "quickget.exe");
        res.set("InternalName", "quickget");
        res.set("LegalCopyright", "Copyright © 2026 QuickGet");
        res.set_manifest(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
    <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
        <security>
            <requestedPrivileges>
                <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
            </requestedPrivileges>
        </security>
    </trustInfo>
    <application xmlns="urn:schemas-microsoft-com:asm.v3">
        <windowsSettings>
            <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
            <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2, PerMonitor</dpiAwareness>
            <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
        </windowsSettings>
    </application>
    <dependency>
        <dependentAssembly>
            <assemblyIdentity
                type="win32"
                name="Microsoft.Windows.Common-Controls"
                version="6.0.0.0"
                processorArchitecture="*"
                publicKeyToken="6595b64144ccf1df"
                language="*"
            />
        </dependentAssembly>
    </dependency>
</assembly>
"#,
        );
        if let Err(e) = res.compile() {
            eprintln!("winres error: {e}");
        }
    }
}
