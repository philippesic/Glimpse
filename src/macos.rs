use eframe::egui;
use objc2::{
    runtime::{AnyObject, Imp, Sel},
    sel,
};
use objc2_app_kit::NSApplication;
use objc2_foundation::MainThreadMarker;
use std::sync::OnceLock;

type Terminate = extern "C" fn(&NSApplication, Sel, Option<&AnyObject>);

static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

extern "C" fn request_quit(_app: &NSApplication, _selector: Sel, _sender: Option<&AnyObject>) {
    if let Some(ctx) = CONTEXT.get() {
        close_window(ctx);
    }
}
pub fn close_window(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    ctx.request_repaint();
}

pub fn guard_native_quit(ctx: &egui::Context) {
    if CONTEXT.set(ctx.clone()).is_err() {
        return;
    }
    let mtm = MainThreadMarker::new().expect("The native app runs on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    let method = app
        .class()
        .instance_method(sel!(terminate:))
        .expect("NSApplication has terminate:");
    // Winit's menu and Dock Quit otherwise terminate without a window-close event.
    // This process owns NSApplication; every normal quit must reach the buffer guard.
    unsafe {
        let callback: Terminate = request_quit;
        method.set_implementation(std::mem::transmute::<Terminate, Imp>(callback));
    }
}

/// Copy a lossless file URL and, when representable, the ordinary native path text.
pub fn copy_path(path: &std::path::Path) -> Result<(), String> {
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSString, NSURL};
    use std::os::unix::ffi::OsStrExt;
    let value = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let url = unsafe {
        NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
            std::ptr::NonNull::new(value.as_ptr().cast_mut()).unwrap(),
            false,
            None,
        )
    };
    let writer: &ProtocolObject<dyn NSPasteboardWriting> = ProtocolObject::from_ref(&*url);
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    if !board.writeObjects(&NSArray::from_slice(&[writer])) {
        return Err("The system clipboard refused the file URL".into());
    }
    if let Some(text) = path.to_str()
        && !board.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString })
    {
        return Err("The file URL was copied but path text could not be added".into());
    }
    Ok(())
}
