use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::transport::bridge::AegisError;

const DISPLAY_WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const VNC_RESOLUTION: &str = "1440x960x24";

#[derive(Debug, Clone, Serialize)]
pub struct DashboardBootstrap {
    pub websocket_path: String,
    pub can_reconnect: bool,
}

pub struct LinuxDisplayStack {
    display: String,
    vnc_addr: SocketAddr,
    xvfb: Child,
    x11vnc: Child,
}

impl LinuxDisplayStack {
    pub fn display(&self) -> &str {
        &self.display
    }

    pub fn vnc_addr(&self) -> SocketAddr {
        self.vnc_addr
    }

    pub fn bootstrap(&self) -> DashboardBootstrap {
        DashboardBootstrap {
            websocket_path: "/ui/vnc".into(),
            can_reconnect: true,
        }
    }
}

impl Drop for LinuxDisplayStack {
    fn drop(&mut self) {
        let _ = self.x11vnc.kill();
        let _ = self.x11vnc.wait();
        let _ = self.xvfb.kill();
        let _ = self.xvfb.wait();
    }
}

#[cfg(target_os = "linux")]
pub fn spawn_linux_display_stack() -> Result<LinuxDisplayStack, AegisError> {
    let display = reserve_display_name()?;
    let vnc_port = bind_ephemeral_port()?;
    let vnc_addr = SocketAddr::from(([127, 0, 0, 1], vnc_port));
    let log_dir = std::env::temp_dir().join("aegis");
    std::fs::create_dir_all(&log_dir)?;

    let xvfb_log = std::fs::File::create(log_dir.join("xvfb.log"))?;
    let x11vnc_log = std::fs::File::create(log_dir.join("x11vnc.log"))?;

    let xvfb = Command::new("Xvfb")
        .arg(&display)
        .args(["-screen", "0", VNC_RESOLUTION, "-nolisten", "tcp", "-ac"])
        .stdout(Stdio::from(xvfb_log.try_clone()?))
        .stderr(Stdio::from(xvfb_log))
        .spawn()
        .map_err(|error| AegisError::Bridge(format!("failed to start Xvfb: {error}")))?;

    wait_for_display_socket(&display)?;

    let x11vnc = Command::new("x11vnc")
        .arg("-display")
        .arg(&display)
        .args([
            "-rfbport",
            &vnc_port.to_string(),
            "-localhost",
            "-forever",
            "-shared",
            "-nopw",
            "-quiet",
            "-noxdamage",
        ])
        .stdout(Stdio::from(x11vnc_log.try_clone()?))
        .stderr(Stdio::from(x11vnc_log))
        .spawn()
        .map_err(|error| AegisError::Bridge(format!("failed to start x11vnc: {error}")))?;

    wait_for_tcp(vnc_addr)?;

    Ok(LinuxDisplayStack {
        display,
        vnc_addr,
        xvfb,
        x11vnc,
    })
}

#[cfg(not(target_os = "linux"))]
pub fn spawn_linux_display_stack() -> Result<LinuxDisplayStack, AegisError> {
    Err(AegisError::Bridge(
        "the Linux dashboard display stack is only available on Linux".into(),
    ))
}

#[cfg(target_os = "linux")]
pub fn set_display_env(display: &str) {
    unsafe {
        std::env::set_var("DISPLAY", display);
    }
}

#[cfg(not(target_os = "linux"))]
pub fn set_display_env(_display: &str) {}

pub fn open_dashboard(url: &str) -> Result<(), AegisError> {
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                AegisError::Bridge(format!("failed to open dashboard browser: {error}"))
            })?;
        return Ok(());
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = url;
        Err(AegisError::Bridge(
            "dashboard opening is only implemented on Linux".into(),
        ))
    }
}

fn bind_ephemeral_port() -> Result<u16, AegisError> {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    Ok(listener.local_addr()?.port())
}

fn reserve_display_name() -> Result<String, AegisError> {
    for number in 90..200 {
        let socket = PathBuf::from(format!("/tmp/.X11-unix/X{number}"));
        let lock = PathBuf::from(format!("/tmp/.X{number}-lock"));
        if !socket.exists() && !lock.exists() {
            return Ok(format!(":{number}"));
        }
    }
    Err(AegisError::Bridge(
        "failed to reserve a free X display for the Linux dashboard".into(),
    ))
}

fn wait_for_display_socket(display: &str) -> Result<(), AegisError> {
    let display_number = display.trim_start_matches(':');
    let socket = PathBuf::from(format!("/tmp/.X11-unix/X{display_number}"));
    let deadline = Instant::now() + DISPLAY_WAIT_TIMEOUT;
    while Instant::now() < deadline {
        if socket.exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(AegisError::Bridge(format!(
        "timed out waiting for Xvfb display socket {socket:?}"
    )))
}

fn wait_for_tcp(addr: SocketAddr) -> Result<(), AegisError> {
    let deadline = Instant::now() + DISPLAY_WAIT_TIMEOUT;
    while Instant::now() < deadline {
        if TcpListener::bind(addr).is_err() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(AegisError::Bridge(format!(
        "timed out waiting for VNC listener on {addr}"
    )))
}
