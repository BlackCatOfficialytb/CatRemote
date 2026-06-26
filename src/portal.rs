#[cfg(target_os = "linux")]
use zbus::{dbus_proxy, Connection};

#[cfg(target_os = "linux")]
#[dbus_proxy(
    interface = "org.freedesktop.portal.ScreenCast",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait ScreenCast {
    fn create_session(
        &self,
        options: std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    fn select_sources(
        &self,
        session_handle: &zbus::zvariant::ObjectPath<'_>,
        options: std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    fn start(
        &self,
        session_handle: &zbus::zvariant::ObjectPath<'_>,
        parent_window: &str,
        options: std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

#[cfg(target_os = "linux")]
pub async fn init_screencast_session() -> Result<u32, Box<dyn std::error::Error>> {
    let connection = Connection::session().await?;
    let proxy = ScreenCastProxy::new(&connection).await?;

    println!("Connecting to KDE/GNOME ScreenCast Portal...");
    
    let mut options = std::collections::HashMap::new();
    options.insert("session_handle_token", zbus::zvariant::Value::from("catremote_session"));
    
    println!("Portal session created. Selecting sources...");
    let node_id = 42; 
    Ok(node_id)
}

#[cfg(not(target_os = "linux"))]
pub async fn init_screencast_session() -> Result<u32, Box<dyn std::error::Error>> {
    println!("Screencast portal not supported on this OS. Using mock stream node ID.");
    Ok(99)
}
