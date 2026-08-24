//! 从终端启动时，给 Dock 注入应用图标。

use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

pub fn set_dock_icon() {
    static ICON_BYTES: &[u8] = include_bytes!("../../../assets/icon-512.png");
    unsafe {
        let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
        let data: *mut Object = msg_send![
            class!(NSData),
            dataWithBytes: ICON_BYTES.as_ptr() as *const std::ffi::c_void
            length: ICON_BYTES.len()
        ];
        if !data.is_null() {
            let image_alloc: *mut Object = msg_send![class!(NSImage), alloc];
            let image: *mut Object = msg_send![image_alloc, initWithData: data];
            if !image.is_null() {
                let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
                if !app.is_null() {
                    let _: () = msg_send![app, setApplicationIconImage: image];
                }
                let _: () = msg_send![image, release];
            }
        }
        let _: () = msg_send![pool, drain];
    }
}
