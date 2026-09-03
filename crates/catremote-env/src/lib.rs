use anyhow::Result;
use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
use drm::control::Device as DrmDevice;

#[cfg(target_os = "linux")]
use libva::{Display as VaDisplay, VA_STATUS_SUCCESS};

#[cfg(target_os = "linux")]
use nvml_wrapper::Nvml;

#[cfg(target_os = "linux")]
use pipewire as pw;

#[cfg(target_os = "linux")]
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

#[cfg(target_os = "linux")]
use wayland_protocols::wlr::unstable::screencopy::v1::client::wlr_screencopy_manager_v1::WlrScreencopyManagerV1;

#[cfg(target_os = "linux")]
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_manager_v1::ExtImageCaptureSourceManagerV1;

#[cfg(target_os = "linux")]
use zbus::{Connection as ZbusConnection, proxy};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Capabilities {
    pub compositor: CompositorCapabilities,
    pub portal: PortalCapabilities,
    pub gpu_encoders: GpuEncoderCapabilities,
    pub pipewire: PipeWireCapabilities,
    pub kernel: KernelCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompositorCapabilities {
    pub wlr_screencopy_v1: bool,
    pub ext_image_capture_source_v1: bool,
    pub compositor_name: Option<String>,
    pub compositor_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PortalCapabilities {
    pub screencast_available: bool,
    pub screencast_permission: PortalPermissionState,
    pub portal_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PortalPermissionState {
    #[default]
    Unknown,
    Granted,
    Denied,
    NotAvailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GpuEncoderCapabilities {
    pub vaapi: VaapiCapabilities,
    pub nvenc: NvencCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaapiCapabilities {
    pub available: bool,
    pub driver: Option<String>,
    pub version: Option<String>,
    pub supported_profiles: Vec<VaapiProfile>,
    pub supported_entrypoints: Vec<VaapiEntrypoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub struct VaapiProfile {
    pub name: String,
    pub profile_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub struct VaapiEntrypoint {
    pub name: String,
    pub entrypoint_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NvencCapabilities {
    pub available: bool,
    pub driver_version: Option<String>,
    pub gpu_name: Option<String>,
    pub supported_codecs: Vec<NvencCodec>,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub struct NvencCodec {
    pub name: String,
    pub codec_guid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipeWireCapabilities {
    pub available: bool,
    pub version: Option<String>,
    pub required_modules: Vec<String>,
    pub missing_modules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KernelCapabilities {
    pub dma_buf: DmaBufCapabilities,
    pub libei: LibeiCapabilities,
    pub evdev: EvdevCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DmaBufCapabilities {
    pub available: bool,
    pub heaps: Vec<DmaHeap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DmaHeap {
    pub name: String,
    pub path: String,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LibeiCapabilities {
    pub available: bool,
    pub seats: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvdevCapabilities {
    pub available: bool,
    pub devices: Vec<EvdevDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvdevDevice {
    pub path: String,
    pub name: String,
    pub capabilities: Vec<String>,
}

pub async fn detect_capabilities() -> Result<Capabilities> {
    #[cfg(target_os = "linux")]
    {
        let mut caps = Capabilities::default();

        caps.compositor = detect_compositor().await?;
        caps.portal = detect_portal().await?;
        caps.gpu_encoders = detect_gpu_encoders().await?;
        caps.pipewire = detect_pipewire().await?;
        caps.kernel = detect_kernel().await?;

        Ok(caps)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(Capabilities::default())
    }
}

#[cfg(target_os = "linux")]
async fn detect_compositor() -> Result<CompositorCapabilities> {
    let mut caps = CompositorCapabilities::default();

    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(_) => return Ok(caps),
    };

    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let display = conn.display();
    let registry = display.get_registry(&qh, ());

    event_queue.roundtrip(&mut ()).ok();

    let globals = registry.contents::<wayland_client::protocol::wl_registry::WlRegistry>(&qh);
    if let Some(reg) = globals {
        for (name, interface, version) in reg.globals() {
            match interface.as_str() {
                "wlr_screencopy_manager_v1" => {
                    caps.wlr_screencopy_v1 = true;
                }
                "ext_image_capture_source_manager_v1" => {
                    caps.ext_image_capture_source_v1 = true;
                }
                "wl_compositor" => {
                    caps.compositor_name = Some("wl_compositor".to_string());
                    caps.compositor_version = Some(version.to_string());
                }
                _ => {}
            }
        }
    }

    Ok(caps)
}

#[cfg(not(target_os = "linux"))]
async fn detect_compositor() -> Result<CompositorCapabilities> {
    Ok(CompositorCapabilities::default())
}

#[cfg(target_os = "linux")]
async fn detect_portal() -> Result<PortalCapabilities> {
    let mut caps = PortalCapabilities::default();

    let conn = match ZbusConnection::session().await {
        Ok(c) => c,
        Err(_) => return Ok(caps),
    };

    #[proxy(
        interface = "org.freedesktop.portal.Desktop",
        default_service = "org.freedesktop.portal.Desktop",
        default_path = "/org/freedesktop/portal/desktop"
    )]
    trait Desktop {
        async fn request_permission(
            &self,
            handle: &str,
            app_id: &str,
            permission: &str,
            options: &zvariant::Dict<'_, &str, &zvariant::Value<'_>>,
        ) -> zbus::Result<u32>;
    }

    let proxy = DesktopProxy::new(&conn).await;
    if proxy.is_ok() {
        caps.screencast_available = true;
        caps.portal_version = Some("1.0".to_string());

        let proxy = proxy.unwrap();
        let options = zvariant::Dict::new(&[]);
        let result = proxy.request_permission("", "catremote", "screencast", &options).await;
        match result {
            Ok(0) => caps.screencast_permission = PortalPermissionState::Granted,
            Ok(1) => caps.screencast_permission = PortalPermissionState::Denied,
            Ok(_) => caps.screencast_permission = PortalPermissionState::Unknown,
            Err(_) => caps.screencast_permission = PortalPermissionState::NotAvailable,
        }
    }

    Ok(caps)
}

#[cfg(not(target_os = "linux"))]
async fn detect_portal() -> Result<PortalCapabilities> {
    Ok(PortalCapabilities::default())
}

#[cfg(target_os = "linux")]
async fn detect_gpu_encoders() -> Result<GpuEncoderCapabilities> {
    let mut caps = GpuEncoderCapabilities::default();

    caps.vaapi = detect_vaapi().await?;
    caps.nvenc = detect_nvenc().await?;

    Ok(caps)
}

#[cfg(target_os = "linux")]
async fn detect_vaapi() -> Result<VaapiCapabilities> {
    let mut caps = VaapiCapabilities::default();

    let display = match VaDisplay::open() {
        Ok(d) => d,
        Err(_) => return Ok(caps),
    };

    caps.available = true;

    let mut major = 0;
    let mut minor = 0;
    if unsafe { libva::vaInitialize(display.as_raw(), &mut major, &mut minor) } == VA_STATUS_SUCCESS {
        caps.version = Some(format!("{}.{}", major, minor));
    }

    if let Ok(driver) = display.get_driver_string() {
        caps.driver = Some(driver.to_string_lossy().to_string());
    }

    let profiles = display.query_config_profiles();
    for profile in profiles {
        caps.supported_profiles.push(VaapiProfile {
            name: format!("{:?}", profile),
            profile_id: profile as i32,
        });
    }

    let entrypoints = display.query_config_entrypoints();
    for entrypoint in entrypoints {
        caps.supported_entrypoints.push(VaapiEntrypoint {
            name: format!("{:?}", entrypoint),
            entrypoint_id: entrypoint as i32,
        });
    }

    Ok(caps)
}

#[cfg(target_os = "linux")]
async fn detect_nvenc() -> Result<NvencCapabilities> {
    let mut caps = NvencCapabilities::default();

    let nvml = match Nvml::init() {
        Ok(n) => n,
        Err(_) => return Ok(caps),
    };

    caps.available = true;

    if let Ok(version) = nvml.sys_driver_version() {
        caps.driver_version = Some(version);
    }

    let device_count = nvml.device_count().unwrap_or(0);
    if device_count > 0 {
        if let Ok(device) = nvml.device_by_index(0) {
            if let Ok(name) = device.name() {
                caps.gpu_name = Some(name);
            }
        }
    }

    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=encoder_stats.sessionCount", "--format=csv,noheader,nounits"])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let codecs = String::from_utf8_lossy(&out.stdout);
            for line in codecs.lines() {
                if !line.trim().is_empty() {
                    caps.supported_codecs.push(NvencCodec {
                        name: line.trim().to_string(),
                        codec_guid: "".to_string(),
                    });
                }
            }
        }
    }

    caps.max_width = Some(8192);
    caps.max_height = Some(8192);

    Ok(caps)
}

#[cfg(target_os = "linux")]
async fn detect_pipewire() -> Result<PipeWireCapabilities> {
    let mut caps = PipeWireCapabilities::default();

    let output = Command::new("pipewire")
        .arg("--version")
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let version_str = String::from_utf8_lossy(&out.stdout);
            caps.version = Some(version_str.trim().to_string());
            caps.available = true;

            if let Some(ver) = parse_version(&version_str) {
                if ver >= (0, 3, 70) {
                    caps.required_modules = vec![
                        "libpipewire-module-libcamera".to_string(),
                        "libpipewire-module-roc".to_string(),
                        "libpipewire-module-v4l2".to_string(),
                        "libpipewire-module-x11-bell".to_string(),
                    ];
                    for module in &caps.required_modules.clone() {
                        if !check_pipewire_module(module).await {
                            caps.missing_modules.push(module.clone());
                        }
                    }
                }
            }
        }
    }

    Ok(caps)
}

fn parse_version(version_str: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = version_str.split('.').collect();
    if parts.len() >= 3 {
        let major = parts[0].parse().ok()?;
        let minor = parts[1].parse().ok()?;
        let patch = parts[2].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
        Some((major, minor, patch))
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
async fn check_pipewire_module(module: &str) -> bool {
    let output = Command::new("pw-cli")
        .args(["info", "all"])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let info = String::from_utf8_lossy(&out.stdout);
            return info.contains(module);
        }
    }
    false
}

#[cfg(target_os = "linux")]
async fn detect_kernel() -> Result<KernelCapabilities> {
    let mut caps = KernelCapabilities::default();

    caps.dma_buf = detect_dma_buf().await?;
    caps.libei = detect_libei().await?;
    caps.evdev = detect_evdev().await?;

    Ok(caps)
}

#[cfg(target_os = "linux")]
async fn detect_dma_buf() -> Result<DmaBufCapabilities> {
    let mut caps = DmaBufCapabilities::default();

    let dma_heap_path = Path::new("/dev/dma_heap");
    if dma_heap_path.exists() {
        caps.available = true;
        if let Ok(entries) = std::fs::read_dir(dma_heap_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    let size = std::fs::metadata(&path).ok().map(|m| m.len());
                    caps.heaps.push(DmaHeap {
                        name: name.to_string(),
                        path: path.to_string_lossy().to_string(),
                        size,
                    });
                }
            }
        }
    }

    Ok(caps)
}

#[cfg(target_os = "linux")]
async fn detect_libei() -> Result<LibeiCapabilities> {
    let mut caps = LibeiCapabilities::default();

    let output = Command::new("libei")
        .arg("--version")
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            caps.available = true;
            let version_str = String::from_utf8_lossy(&out.stdout);
            caps.seats.push(version_str.trim().to_string());
        }
    }

    if let Ok(seats) = std::fs::read_dir("/run/user/") {
        for entry in seats.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let libei_socket = path.join("libei");
                if libei_socket.exists() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        caps.seats.push(format!("seat-{}", name));
                    }
                }
            }
        }
    }

    Ok(caps)
}

#[cfg(target_os = "linux")]
async fn detect_evdev() -> Result<EvdevCapabilities> {
    let mut caps = EvdevCapabilities::default();

    let input_path = Path::new("/dev/input");
    if input_path.exists() {
        caps.available = true;
        if let Ok(entries) = std::fs::read_dir(input_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("event") {
                        let device_name = get_evdev_name(&path).unwrap_or_else(|| "unknown".to_string());
                        let capabilities = get_evdev_capabilities(&path).unwrap_or_default();
                        caps.devices.push(EvdevDevice {
                            path: path.to_string_lossy().to_string(),
                            name: device_name,
                            capabilities,
                        });
                    }
                }
            }
        }
    }

    Ok(caps)
}

#[cfg(target_os = "linux")]
fn get_evdev_name(path: &Path) -> Option<String> {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;
    use std::os::unix::io::AsRawFd;

    let fd = open(path, OFlag::O_RDONLY, Mode::empty()).ok()?;
    let mut name = [0u8; 256];
    let result = unsafe { libc::ioctl(fd.as_raw_fd(), libc::EVIOCGNAME(256), name.as_mut_ptr()) };
    if result >= 0 {
        let len = name.iter().position(|&c| c == 0).unwrap_or(256);
        Some(String::from_utf8_lossy(&name[..len]).to_string())
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn get_evdev_capabilities(path: &Path) -> Option<Vec<String>> {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;
    use std::os::unix::io::AsRawFd;

    let fd = open(path, OFlag::O_RDONLY, Mode::empty()).ok()?;
    let mut caps = [0u8; 256];
    let result = unsafe { libc::ioctl(fd.as_raw_fd(), libc::EVIOCGBIT(0, 256), caps.as_mut_ptr()) };
    if result >= 0 {
        let mut capability_names = Vec::new();
        for (i, &byte) in caps.iter().enumerate() {
            if byte != 0 {
                for bit in 0..8 {
                    if byte & (1 << bit) != 0 {
                        let event_type = i * 8 + bit;
                        capability_names.push(event_type_to_string(event_type));
                    }
                }
            }
        }
        Some(capability_names)
    } else {
        None
    }
}

fn event_type_to_string(event_type: usize) -> String {
    match event_type {
        0 => "EV_SYN".to_string(),
        1 => "EV_KEY".to_string(),
        2 => "EV_REL".to_string(),
        3 => "EV_ABS".to_string(),
        4 => "EV_MSC".to_string(),
        5 => "EV_SW".to_string(),
        17 => "EV_LED".to_string(),
        18 => "EV_SND".to_string(),
        20 => "EV_REP".to_string(),
        21 => "EV_FF".to_string(),
        22 => "EV_PWR".to_string(),
        23 => "EV_FF_STATUS".to_string(),
        _ => format!("EV_UNKNOWN({})", event_type),
    }
}

#[cfg(not(target_os = "linux"))]
async fn detect_gpu_encoders() -> Result<GpuEncoderCapabilities> {
    Ok(GpuEncoderCapabilities::default())
}

#[cfg(not(target_os = "linux"))]
async fn detect_vaapi() -> Result<VaapiCapabilities> {
    Ok(VaapiCapabilities::default())
}

#[cfg(not(target_os = "linux"))]
async fn detect_nvenc() -> Result<NvencCapabilities> {
    Ok(NvencCapabilities::default())
}

#[cfg(not(target_os = "linux"))]
async fn detect_pipewire() -> Result<PipeWireCapabilities> {
    Ok(PipeWireCapabilities::default())
}

#[cfg(not(target_os = "linux"))]
async fn check_pipewire_module(_module: &str) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
async fn detect_kernel() -> Result<KernelCapabilities> {
    Ok(KernelCapabilities::default())
}

#[cfg(not(target_os = "linux"))]
async fn detect_dma_buf() -> Result<DmaBufCapabilities> {
    Ok(DmaBufCapabilities::default())
}

#[cfg(not(target_os = "linux"))]
async fn detect_libei() -> Result<LibeiCapabilities> {
    Ok(LibeiCapabilities::default())
}

#[cfg(not(target_os = "linux"))]
async fn detect_evdev() -> Result<EvdevCapabilities> {
    Ok(EvdevCapabilities::default())
}