//! Async IPC transport (Tokio Unix socket server and client).
//! Command/response vocabulary and validation live in `wallr-common`.

pub use wallr_common::ipc::*;

use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

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

async fn write_response(writer: &mut tokio::io::WriteHalf<UnixStream>, response: &IpcResponse) {
    if let Ok(res_data) = serde_json::to_vec(response) {
        let _ = writer.write_all(&res_data).await;
        let _ = writer.write_all(b"\n").await;
        let _ = writer.flush().await;
    }
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
                        let reader = BufReader::new(reader);
                        // Bound the request: `take` caps memory even when a
                        // client never sends a newline.
                        let mut limited = reader.take((MAX_IPC_BYTES + 1) as u64);
                        let mut buf = Vec::with_capacity(1024);
                        let response = match limited.read_until(b'\n', &mut buf).await {
                            Ok(0) => return,
                            Ok(_) if buf.len() > MAX_IPC_BYTES => {
                                IpcResponse::err(format!("request exceeds {MAX_IPC_BYTES} bytes"))
                            }
                            Ok(_) => match serde_json::from_slice::<IpcCommand>(&buf) {
                                Ok(cmd) => match validate_command(&cmd) {
                                    Ok(()) => handler_clone(cmd).await,
                                    Err(reason) => IpcResponse::err(reason),
                                },
                                Err(e) => IpcResponse::err(format!("invalid IPC command: {e}")),
                            },
                            Err(e) => IpcResponse::err(format!("IPC read error: {e}")),
                        };
                        write_response(&mut writer, &response).await;
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
