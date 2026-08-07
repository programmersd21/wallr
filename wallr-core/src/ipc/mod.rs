use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum IpcCommand {
    Pause {
        #[serde(default)]
        monitor: Option<String>,
    },
    Resume {
        #[serde(default)]
        monitor: Option<String>,
    },
    Reload,
    Seek {
        timestamp_ms: u64,
        #[serde(default)]
        monitor: Option<String>,
    },
    Preview {
        path: String,
        effect: Option<crate::animation::Effect>,
        duration_ms: Option<u32>,
        #[serde(default)]
        no_theme: bool,
        #[serde(default)]
        theme_override: Option<crate::config::ThemeProvider>,
        #[serde(default)]
        monitor: Option<String>,
        #[serde(default)]
        scaling_mode: Option<crate::config::ScalingMode>,
    },
    Stop,
    Status,
    Info {
        #[serde(default)]
        monitor: Option<String>,
    },
    MonitorList,
    MonitorCurrent,
    Blank {
        #[serde(default)]
        monitor: Option<String>,
        #[serde(default)]
        effect: Option<crate::animation::Effect>,
        #[serde(default)]
        duration_ms: Option<u32>,
    },
    Restore {
        #[serde(default)]
        monitor: Option<String>,
        #[serde(default)]
        effect: Option<crate::animation::Effect>,
        #[serde(default)]
        duration_ms: Option<u32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub success: bool,
    pub message: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("daemon not running: {0}")]
    DaemonNotRunning(String),
    #[error("IPC I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol serialization/deserialization error: {0}")]
    Protocol(#[from] serde_json::Error),
}

pub async fn send_ipc_command<P: AsRef<Path>>(
    socket_path: P,
    command: IpcCommand,
) -> Result<IpcResponse, IpcError> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| IpcError::DaemonNotRunning(e.to_string()))?;

    let req_data = serde_json::to_vec(&command)?;
    stream.write_all(&req_data).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;

    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    reader.read_line(&mut response_line).await?;

    let response: IpcResponse = serde_json::from_str(&response_line)?;
    Ok(response)
}

pub async fn start_ipc_server<P, F, Fut>(socket_path: P, handler: F) -> Result<(), IpcError>
where
    P: AsRef<Path>,
    F: Fn(IpcCommand) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = IpcResponse> + Send + 'static,
{
    let path = socket_path.as_ref();
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }

    let listener = UnixListener::bind(path)?;
    let handler = std::sync::Arc::new(handler);

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let handler_clone = handler.clone();
                    tokio::spawn(async move {
                        let (reader, mut writer) = tokio::io::split(stream);
                        let mut reader = BufReader::new(reader);
                        let mut line = String::new();

                        if let Ok(n) = reader.read_line(&mut line).await
                            && n > 0
                            && let Ok(cmd) = serde_json::from_str::<IpcCommand>(&line)
                        {
                            let response = handler_clone(cmd).await;
                            if let Ok(res_data) = serde_json::to_vec(&response) {
                                let _ = writer.write_all(&res_data).await;
                                let _ = writer.write_all(b"\n").await;
                                let _ = writer.flush().await;
                            }
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("IPC accept error: {:?}", e);
                }
            }
        }
    });

    Ok(())
}
