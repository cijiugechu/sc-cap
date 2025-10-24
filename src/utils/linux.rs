pub fn is_supported() -> bool {
    // Best-effort: ensure a session bus exists and that the xdg-desktop-portal
    // ScreenCast interface can be reached. This avoids panics in portal calls
    // when the service is missing.
    // Also require a graphical session (X11/Wayland) environment.
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        return false;
    }
    if let Ok(conn) = dbus::blocking::Connection::new_session() {
        let proxy = conn.with_proxy(
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            std::time::Duration::from_millis(500),
        );
        // Probe availability by querying AvailableSourceTypes; a non-zero value indicates screens/windows supported
        let available: Result<u32, _> = <dbus::blocking::Proxy<_> as dbus::blocking::stdintf::org_freedesktop_dbus::Properties>::get(
            &proxy,
            "org.freedesktop.portal.ScreenCast",
            "AvailableSourceTypes",
        );
        if let Ok(mask) = available { return mask != 0; }
        return false;
    }
    false
}

pub fn has_permission() -> bool {
    // On Linux, permission is mediated interactively by the portal; just return true
    // so callers proceed to invoke the portal dialog.
    true
}
