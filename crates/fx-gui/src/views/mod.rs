//! One module per screen region. Each renders `fx-app` state and calls its methods.

pub mod about;
pub mod commandbar;
pub mod dialogs;
pub mod dnd;
pub mod icons;
pub mod menu;
pub mod menubar;
#[cfg(any(windows, target_os = "macos"))]
pub mod native_drag;
pub mod palette;
pub mod settings;
pub mod sidebar;
pub mod status;
pub mod table;
pub mod tabs;
pub mod toast;
pub mod toolbar;
pub mod transfers;
#[cfg(all(unix, not(target_os = "macos")))]
pub mod xdnd;
