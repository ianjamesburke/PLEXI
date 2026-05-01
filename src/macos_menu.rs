use objc2::declare::ClassBuilder;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{sel, ClassType};
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
use objc2_foundation::{MainThreadMarker, NSString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// Wrapper to store a raw pointer to the handler in a static.
/// Safety: the handler is only created/accessed on the main thread.
struct JsonHandler(*const AnyObject);
unsafe impl Send for JsonHandler {}
unsafe impl Sync for JsonHandler {}

static HANDLER: OnceLock<JsonHandler> = OnceLock::new();

/// Set by the "Reload Configuration" menu item; polled each frame in
/// `PlexiApp::update()`.
static RELOAD_CONFIG_FLAG: AtomicBool = AtomicBool::new(false);

/// Check and clear the reload flag. Called from the main app loop.
pub fn take_reload_config_flag() -> bool {
    RELOAD_CONFIG_FLAG.swap(false, Ordering::SeqCst)
}

fn register_handler_class() -> &'static objc2::runtime::AnyClass {
    static CLASS: OnceLock<&'static objc2::runtime::AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = NSObject::class();
        let mut builder =
            ClassBuilder::new("PlexiMenuHandler", superclass).expect("class already registered");

        unsafe extern "C" fn open_config(_this: &AnyObject, _cmd: Sel, _sender: *mut AnyObject) {
            crate::config::open_config_file();
        }

        unsafe extern "C" fn reload_config(_this: &AnyObject, _cmd: Sel, _sender: *mut AnyObject) {
            RELOAD_CONFIG_FLAG.store(true, Ordering::SeqCst);
        }

        unsafe {
            builder.add_method(
                sel!(openConfig:),
                open_config as unsafe extern "C" fn(_, _, _),
            );
            builder.add_method(
                sel!(reloadConfig:),
                reload_config as unsafe extern "C" fn(_, _, _),
            );
        }

        builder.register()
    })
}

fn get_handler() -> *const AnyObject {
    HANDLER
        .get_or_init(|| {
            let cls = register_handler_class();
            let obj: Retained<NSObject> = unsafe { objc2::msg_send_id![cls, new] };
            // Leak the object so the menu item's target stays alive forever.
            let ptr: *const AnyObject = Retained::as_ptr(&obj) as *const AnyObject;
            std::mem::forget(obj);
            JsonHandler(ptr)
        })
        .0
}

/// Remove intercepted menu items (Hide, Hide Others, Quit) from the default
/// macOS app menu so our app can use Cmd+H and Cmd+Q directly, then add
/// "Open Config" item.
pub fn customize_app_menu() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("Not on main thread, cannot modify macOS menu");
        return;
    };

    let app = NSApplication::sharedApplication(mtm);
    let Some(main_menu) = (unsafe { app.mainMenu() }) else {
        return;
    };

    // First submenu (index 0) is the app menu containing Hide/Quit/etc.
    let Some(app_menu_item) = (unsafe { main_menu.itemAtIndex(0) }) else {
        return;
    };
    let Some(app_menu) = (unsafe { app_menu_item.submenu() }) else {
        return;
    };

    remove_items_with_action(&app_menu, sel!(hide:));
    // Also remove "Hide Others" (Cmd+Opt+H) to avoid confusion
    remove_items_with_action(&app_menu, sel!(hideOtherApplications:));
    // Remove "Quit" (Cmd+Q) so it reaches egui's consume_key for proper save-on-quit
    remove_items_with_action(&app_menu, sel!(terminate:));

    // Add "Reload Config" and "Open Config" items at the top of the app menu
    let handler = get_handler();

    let reload_title = NSString::from_str("Reload Configuration");
    let reload_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &reload_title,
            Some(sel!(reloadConfig:)),
            &NSString::from_str(","),
        )
    };
    // NSEventModifierFlagCommand (1<<20) | NSEventModifierFlagShift (1<<17)
    unsafe {
        let mask: usize = (1 << 20) | (1 << 17);
        let _: () = objc2::msg_send![&*reload_item, setKeyEquivalentModifierMask: mask];
    };
    unsafe { reload_item.setTarget(Some(&*handler)) };
    app_menu.addItem(&reload_item);

    let title = NSString::from_str("Open Config\u{2026}");
    let key_equiv = NSString::from_str(",");
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &title,
            Some(sel!(openConfig:)),
            &key_equiv,
        )
    };
    unsafe { item.setTarget(Some(&*handler)) };
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    app_menu.addItem(&item);
}

fn remove_items_with_action(menu: &NSMenu, action: objc2::runtime::Sel) {
    let count = unsafe { menu.numberOfItems() };
    for i in (0..count).rev() {
        if let Some(item) = unsafe { menu.itemAtIndex(i) } {
            if unsafe { item.action() } == Some(action) {
                unsafe { menu.removeItemAtIndex(i) };
            }
        }
    }
}
