slint::include_modules!();

use catremote::state::ConnectionState;

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    // Initialize state
    ui.set_connection_state(ConnectionState::Disconnected.to_string().into());

    let ui_handle = ui.as_weak();
    
    ui.on_connect_clicked(move |ip, code| {
        if let Some(ui) = ui_handle.upgrade() {
            println!("Attempting to connect to IP: {}, Code: {}", ip, code);
            ui.set_connection_state(ConnectionState::Connecting.to_string().into());
            
            // TODO: In a real app, this would spawn a background task and update
            // the state upon success. For now, we mock a successful connection.
            let ui_handle_clone = ui.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_secs(1), move || {
                if let Some(ui) = ui_handle_clone.upgrade() {
                    ui.set_connection_state(ConnectionState::Connected.to_string().into());
                }
            });
        }
    });

    let ui_handle = ui.as_weak();
    ui.on_disconnect_clicked(move || {
        if let Some(ui) = ui_handle.upgrade() {
            println!("Disconnecting...");
            ui.set_connection_state(ConnectionState::Disconnected.to_string().into());
        }
    });

    ui.run()
}
