pub fn notify(title: &str, msg: &str) {
    // Deliver off the winit/UI thread. `mac-notification-sys::sendNotification`
    // re-enters the macOS run loop, which panics if called inside a winit
    // event handler (see `winit-0.30 event_handler.rs:135`).
    let title = title.to_owned();
    let msg = msg.to_owned();
    std::thread::spawn(move || {
        #[cfg(target_os = "macos")]
        {
            // On macOS `notify-rust` / `mac-notification-sys` requires an app
            // bundle (CFBundleIdentifier). Without it macOS shows a bogus
            // "Choose Application" dialog and the ObjC call re-enters winit.
            // Use `osascript` exclusively — safe off-thread and bundle-free.
            let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
            let _ = std::process::Command::new("osascript")
                .args([
                    "-e",
                    &format!(
                        "display notification \"{}\" with title \"{}\"",
                        esc(&msg),
                        esc(&title)
                    ),
                ])
                .output();
        }
        #[cfg(not(target_os = "macos"))]
        {
            if notify_rust::Notification::new()
                .summary(&title)
                .body(&msg)
                .timeout(notify_rust::Timeout::Milliseconds(5000))
                .show()
                .is_ok()
            {
                return;
            }
            // fallback for non-macOS (e.g. Linux)
            let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
            let _ = std::process::Command::new("osascript")
                .args([
                    "-e",
                    &format!(
                        "display notification \"{}\" with title \"{}\"",
                        esc(&msg),
                        esc(&title)
                    ),
                ])
                .output();
        }
    });
}
