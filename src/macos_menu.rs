use objc2::sel;
use objc2_app_kit::{NSApplication, NSMenu};
use objc2_foundation::MainThreadMarker;

/// Remove intercepted menu items (Hide, Hide Others, Quit) from the default
/// macOS app menu so our app can use Cmd+H and Cmd+Q directly.
pub fn remove_intercepted_menu_items() {
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
