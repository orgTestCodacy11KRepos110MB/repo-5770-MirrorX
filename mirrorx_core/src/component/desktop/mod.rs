#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::Duplicator;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use self::windows::Duplicator;

mod frame;
pub use frame::CaptureFrame;
pub use frame::Frame;
