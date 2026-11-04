use crate::pairing::RoomSpec;
use anyhow::{bail, Context, Result};
use std::{
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::process::{Child, Command};

pub const SERVICE_PORT: u16 = 29741;

pub struct EngineProcess {
    pub child: Child,
    pub forward_port: u16,
}

/// Re-check the derived room parameters immediately before spawning. The values
/// come from the seed, but a future caller must not be able to smuggle flags or
/// a non-private address into the child process.
pub fn validate(spec: &RoomSpec) -> Result<()> {
    if spec.network_name.is_empty() || spec.network_name.len() > 128 {
        bail!("房间网络名称长度无效");
    }
    if !spec
        .network_name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        bail!("房间网络名称包含无效字符");
    }
    if spec.network_secret.len() < 32 || spec.network_secret.len() > 256 {
        bail!("房间网络密钥长度无效");
    }
    if !["host", "join"].contains(&spec.stored.role.as_str()) {
        bail!("无效的配对角色");
    }
    let local: Ipv4Addr = spec.virtual_ip.parse().context("本机虚拟地址无效")?;
    let peer: Ipv4Addr = spec.peer_ip.parse().context("对方虚拟地址无效")?;
    if !local.is_private() || !peer.is_private() || local == peer {
        bail!("双方需要不同的私有虚拟 IPv4 地址");
    }
    let relay = url::Url::parse(&spec.stored.server.relay_url).context("联机服务器地址无效")?;
    if !["tcp", "udp", "ws", "wss", "quic"].contains(&relay.scheme())
        || relay.host_str().is_none()
        || relay.port().is_none()
        || !relay.username().is_empty()
        || relay.password().is_some()
    {
        bail!("联机服务器地址无效");
    }
    Ok(())
}

pub fn engine_path(resource_dir: &Path) -> PathBuf {
    let name = if cfg!(windows) { "easytier-core.exe" } else { "easytier-core" };
    resource_dir.join("engine").join(name)
}

pub fn spawn_engine(binary: &Path, state_dir: &Path, spec: &RoomSpec) -> Result<EngineProcess> {
    validate(spec)?;
    if !binary.is_file() {
        bail!("未找到联机组件 EasyTier，请先运行 scripts/install_engine.py 安装");
    }
    let machine_file = state_dir.join("machine-id");
    let machine_id = if machine_file.exists() {
        uuid::Uuid::parse_str(std::fs::read_to_string(&machine_file)?.trim())?.to_string()
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        crate::state::write_private(&machine_file, id.as_bytes())?;
        id
    };
    let mut command = Command::new(binary);
    // Allocate distinct loopback ports. EasyTier's automatic RPC port selection
    // races when multiple instances start together, so do not use its port 0 mode.
    let forward_reservation = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let rpc_reservation = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let forward_port = forward_reservation.local_addr()?.port();
    let rpc_port = rpc_reservation.local_addr()?.port();
    // Do not let unrelated global EasyTier environment settings expand this app's scope.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("ET_") {
            command.env_remove(key);
        }
    }
    command
        .arg("--network-name")
        .arg(&spec.network_name)
        .env("ET_NETWORK_SECRET", &spec.network_secret)
        .arg("--ipv4")
        .arg(&spec.virtual_ip)
        .args(["--no-tun", "true", "--use-smoltcp", "true"])
        // The realtime channel must never silently relay file bytes. If hole
        // punching fails, encrypted HTTPS mailbox delivery remains available.
        .args(["--p2p-only", "true"])
        .arg("--peers")
        .arg(&spec.stored.server.relay_url)
        .args(["--listeners", "udp://0.0.0.0:0"])
        .arg("--port-forward")
        .arg(format!(
            "tcp://127.0.0.1:{forward_port}/{}:{SERVICE_PORT}",
            spec.peer_ip
        ))
        .arg("--tcp-whitelist")
        .arg(SERVICE_PORT.to_string())
        .args([
            "--accept-dns",
            "false",
            "--disable-encryption",
            "false",
            "--compression",
            "none",
        ])
        .args(["--bind-device", "false"])
        .arg("--rpc-portal")
        .arg(format!("127.0.0.1:{rpc_port}"))
        .args(["--console-log-level", "error", "--file-log-level", "off"])
        .arg("--machine-id")
        .arg(machine_id)
        .current_dir(state_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    #[cfg(test)]
    command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    drop(forward_reservation);
    drop(rpc_reservation);
    Ok(EngineProcess {
        child: command.spawn().context("无法启动 EasyTier 联机组件")?,
        forward_port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pairing::{test_server, StoredRoom};

    #[tokio::test]
    #[ignore = "requires TWINLINK_ENGINE_TEST_BINARY pointing to official EasyTier 2.6.4"]
    async fn official_engine_accepts_exact_app_configuration() {
        let binary = PathBuf::from(
            std::env::var_os("TWINLINK_ENGINE_TEST_BINARY").expect("set engine binary path"),
        );
        let directory = tempfile::tempdir().unwrap();
        let room = StoredRoom::create(test_server("twinlink-test-prefix"), "join").unwrap();
        let spec = room.spec().unwrap();
        let EngineProcess { mut child, forward_port } =
            spawn_engine(&binary, directory.path(), &spec).unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let alive = child.try_wait().unwrap().is_none();
        let listening = tokio::net::TcpStream::connect(("127.0.0.1", forward_port))
            .await
            .is_ok();
        let _ = child.kill().await;
        assert!(alive, "engine rejected configuration or exited early");
        assert!(listening, "engine did not create its loopback forwarding listener");
    }
}
