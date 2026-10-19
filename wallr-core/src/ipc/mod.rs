//! Async IPC transport (Tokio Unix socket server and client).
//! Command/response vocabulary and validation live in `wallr-common`.

pub use wallr_common::ipc::*;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;

/// Maximum response accepted from a daemon. A status/`info` response grows
/// with the output count but is still far below this bound.
pub const MAX_IPC_RESPONSE_BYTES: usize = 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);
const HANDLER_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_CONCURRENT_CONNECTIONS: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("daemon not running: {0}")]
    DaemonNotRunning(String),
    #[error("daemon already running: {0}")]
    AlreadyRunning(String),
    #[error(
        "refusing to use IPC socket path {0}: it exists but is not a socket owned by this user"
    )]
    UnsafeSocketPath(String),
    #[error("IPC request/response timed out: {0}")]
    Timeout(String),
    #[error("IPC response exceeds {MAX_IPC_RESPONSE_BYTES} bytes")]
    ResponseTooLarge,
    #[error("IPC I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol serialization/deserialization error: {0}")]
    Protocol(#[from] serde_json::Error),
}

/// An exclusively owned, bound Unix-domain IPC socket.
///
/// The advisory lock file serializes daemon startup so two processes can
/// never both decide that the socket is stale. The guard removes the socket
/// on drop, but only when the path still refers to the exact inode created
/// by this instance, so a successor's socket is never unlinked.
pub struct IpcSocket {
    path: PathBuf,
    listener: UnixListener,
    dev: u64,
    ino: u64,
    _lock: std::fs::File,
}

impl IpcSocket {
    /// Acquires the socket path and binds a listener.
    ///
    /// A live socket is reported as [`IpcError::AlreadyRunning`]. A stale
    /// socket (connection refused) is removed and replaced. Any other kind
    /// of filesystem object at the path is left untouched and rejected.
    pub async fn acquire<P: AsRef<Path>>(path: P) -> Result<Self, IpcError> {
        let path = path.as_ref().to_path_buf();
        let lock_path = socket_lock_path(&path);
        let lock = open_lock_file(&lock_path)?;
        lock_exclusive(&lock).map_err(|err| {
            if err.kind() == std::io::ErrorKind::WouldBlock {
                IpcError::AlreadyRunning(path.to_string_lossy().to_string())
            } else {
                IpcError::Io(err)
            }
        })?;

        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                use std::os::unix::fs::{FileTypeExt, MetadataExt};

                if !metadata.file_type().is_socket() {
                    return Err(IpcError::UnsafeSocketPath(
                        path.to_string_lossy().to_string(),
                    ));
                }
                // SAFETY: geteuid has no preconditions and does not touch
                // memory; it only reads the process credentials.
                let euid = unsafe { libc::geteuid() };
                if metadata.uid() != euid {
                    return Err(IpcError::UnsafeSocketPath(
                        path.to_string_lossy().to_string(),
                    ));
                }
                match UnixStream::connect(&path).await {
                    Ok(_) => {
                        return Err(IpcError::AlreadyRunning(path.to_string_lossy().to_string()));
                    }
                    Err(err)
                        if matches!(
                            err.kind(),
                            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                        ) =>
                    {
                        std::fs::remove_file(&path)?;
                    }
                    Err(err) => {
                        // EACCES, EMFILE, and similar conditions are not
                        // proof that the socket is stale, so never unlink it.
                        return Err(IpcError::Io(err));
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(IpcError::Io(err)),
        }

        let listener = UnixListener::bind(&path)?;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if let Err(err) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
            let _ = std::fs::remove_file(&path);
            return Err(IpcError::Io(err));
        }
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) => {
                let _ = std::fs::remove_file(&path);
                return Err(IpcError::Io(err));
            }
        };

        Ok(Self {
            path,
            listener,
            dev: metadata.dev(),
            ino: metadata.ino(),
            _lock: lock,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Serves IPC connections until the listener fails.
    ///
    /// At most [`MAX_CONCURRENT_CONNECTIONS`] connections are handled at
    /// once. Accepting is gated by the semaphore, so a flood backpressures in
    /// the kernel backlog instead of allocating unbounded tasks.
    pub async fn serve<F, Fut>(self, handler: F) -> Result<(), IpcError>
    where
        F: Fn(IpcCommand) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = IpcResponse> + Send + 'static,
    {
        let handler = Arc::new(handler);
        let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
        loop {
            let permit = match Arc::clone(&permits).acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => return Ok(()),
            };
            match self.listener.accept().await {
                Ok((stream, _)) => {
                    let handler = Arc::clone(&handler);
                    tokio::spawn(async move {
                        let _permit = permit;
                        handle_connection(stream, handler).await;
                    });
                }
                Err(err) => {
                    tracing::error!("IPC accept error: {:?}", err);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Runs [`IpcSocket::serve`] on a detached task.
    pub fn spawn<F, Fut>(self, handler: F)
    where
        F: Fn(IpcCommand) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = IpcResponse> + Send + 'static,
    {
        tokio::spawn(async move {
            if let Err(err) = self.serve(handler).await {
                tracing::error!("IPC server stopped: {err}");
            }
        });
    }
}

/// Identity of the bound socket inode, used to clean up safely on paths
/// where `Drop` cannot run (for example a process-wide shutdown handler).
#[derive(Debug, Clone, Copy)]
pub struct IpcSocketIdentity {
    dev: u64,
    ino: u64,
}

impl IpcSocket {
    pub fn identity(&self) -> IpcSocketIdentity {
        IpcSocketIdentity {
            dev: self.dev,
            ino: self.ino,
        }
    }
}

/// Removes `path` only when it is still the exact socket inode identified by
/// `identity`. Returns whether a file was removed.
pub fn remove_socket_if_owned(path: &Path, identity: IpcSocketIdentity) -> bool {
    let owned = std::fs::symlink_metadata(path)
        .is_ok_and(|metadata| existed_as_same_socket(&metadata, identity.dev, identity.ino));
    if owned {
        let _ = std::fs::remove_file(path);
    }
    owned
}

impl Drop for IpcSocket {
    fn drop(&mut self) {
        let _ = remove_socket_if_owned(&self.path, self.identity());
    }
}

fn existed_as_same_socket(metadata: &std::fs::Metadata, dev: u64, ino: u64) -> bool {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    metadata.file_type().is_socket() && metadata.dev() == dev && metadata.ino() == ino
}

fn socket_lock_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("wallr.sock"));
    name.push(".lock");
    path.with_file_name(name)
}

fn open_lock_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
}

fn lock_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: the fd belongs to `file`, which outlives this call. flock only
    // manipulates kernel lock state for that fd and does not access memory.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

async fn handle_connection<F, Fut>(stream: UnixStream, handler: Arc<F>)
where
    F: Fn(IpcCommand) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = IpcResponse> + Send + 'static,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let reader = BufReader::new(reader);
    let mut limited = reader.take((MAX_IPC_BYTES + 1) as u64);
    let mut buf = Vec::with_capacity(1024);
    let response =
        match tokio::time::timeout(REQUEST_TIMEOUT, limited.read_until(b'\n', &mut buf)).await {
            Err(_) => IpcResponse::err("IPC request timed out"),
            Ok(Ok(0)) => return,
            Ok(Ok(_)) if buf.len() > MAX_IPC_BYTES => {
                IpcResponse::err(format!("request exceeds {MAX_IPC_BYTES} bytes"))
            }
            Ok(Ok(_)) => match serde_json::from_slice::<IpcCommand>(&buf) {
                Ok(cmd) => match validate_command(&cmd) {
                    Ok(()) => match tokio::time::timeout(HANDLER_TIMEOUT, handler(cmd)).await {
                        Ok(response) => response,
                        Err(_) => IpcResponse::err("IPC command timed out"),
                    },
                    Err(reason) => IpcResponse::err(reason),
                },
                Err(e) => IpcResponse::err(format!("invalid IPC command: {e}")),
            },
            Ok(Err(e)) => IpcResponse::err(format!("IPC read error: {e}")),
        };
    write_response(&mut writer, &response).await;
}

async fn write_response(writer: &mut tokio::io::WriteHalf<UnixStream>, response: &IpcResponse) {
    let Ok(res_data) = serde_json::to_vec(response) else {
        return;
    };
    let write = async {
        writer.write_all(&res_data).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await
    };
    let _ = tokio::time::timeout(WRITE_TIMEOUT, write).await;
}

async fn read_response(reader: &mut BufReader<UnixStream>) -> Result<Vec<u8>, IpcError> {
    let mut limited = reader.take((MAX_IPC_RESPONSE_BYTES + 1) as u64);
    let mut response_line = Vec::new();
    let read = limited.read_until(b'\n', &mut response_line);
    match tokio::time::timeout(RESPONSE_TIMEOUT, read).await {
        Ok(Ok(0)) => Err(IpcError::Protocol(serde_json::Error::io(
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "daemon closed the connection without a response",
            ),
        ))),
        Ok(Ok(_)) if response_line.len() > MAX_IPC_RESPONSE_BYTES => {
            Err(IpcError::ResponseTooLarge)
        }
        Ok(Ok(_)) => Ok(response_line),
        Ok(Err(err)) => Err(IpcError::Io(err)),
        Err(_) => Err(IpcError::Timeout("waiting for daemon response".to_string())),
    }
}

pub async fn send_ipc_command<P: AsRef<Path>>(
    socket_path: P,
    command: IpcCommand,
) -> Result<IpcResponse, IpcError> {
    let socket_path = socket_path.as_ref();
    let mut stream =
        match tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(socket_path)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(err)) => return Err(IpcError::DaemonNotRunning(err.to_string())),
            Err(_) => {
                return Err(IpcError::Timeout(format!(
                    "connecting to {}",
                    socket_path.display()
                )));
            }
        };

    let req_data = serde_json::to_vec(&command)?;
    let write = async {
        stream.write_all(&req_data).await?;
        stream.write_all(b"\n").await?;
        stream.flush().await
    };
    match tokio::time::timeout(WRITE_TIMEOUT, write).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(IpcError::Io(err)),
        Err(_) => return Err(IpcError::Timeout("sending IPC request".to_string())),
    }

    let mut reader = BufReader::new(stream);
    let response_line = read_response(&mut reader).await?;
    let response: IpcResponse = serde_json::from_slice(&response_line)?;
    Ok(response)
}

/// Bundled server helper kept for callers that do not need to reserve the
/// socket before setting up the rest of the daemon.
pub async fn start_ipc_server<P, F, Fut>(socket_path: P, handler: F) -> Result<(), IpcError>
where
    P: AsRef<Path>,
    F: Fn(IpcCommand) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = IpcResponse> + Send + 'static,
{
    IpcSocket::acquire(socket_path).await?.spawn(handler);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn socket_is_private_and_removed_on_drop() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wallr.sock");
        let socket = IpcSocket::acquire(&path).await.unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        drop(socket);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn live_socket_blocks_a_second_daemon() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wallr.sock");
        let _first = IpcSocket::acquire(&path).await.unwrap();
        assert!(matches!(
            IpcSocket::acquire(&path).await,
            Err(IpcError::AlreadyRunning(_))
        ));
    }

    #[tokio::test]
    async fn stale_socket_is_replaced_but_regular_files_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wallr.sock");
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let socket = IpcSocket::acquire(&path).await.unwrap();
        drop(socket);
        assert!(!path.exists());

        let regular = temp.path().join("not-a-socket");
        std::fs::write(&regular, b"user data").unwrap();
        assert!(matches!(
            IpcSocket::acquire(&regular).await,
            Err(IpcError::UnsafeSocketPath(_))
        ));
        assert_eq!(std::fs::read(&regular).unwrap(), b"user data");
    }

    #[tokio::test]
    async fn server_round_trips_commands_and_rejects_empty_preview_paths() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wallr.sock");
        let socket = IpcSocket::acquire(&path).await.unwrap();
        let server_path = path.clone();
        socket.spawn(|cmd| async move {
            match cmd {
                IpcCommand::Status => IpcResponse {
                    success: true,
                    message: Some("ok".into()),
                },
                other => IpcResponse::err(format!("unexpected {other:?}")),
            }
        });
        let response = send_ipc_command(&server_path, IpcCommand::Status)
            .await
            .unwrap();
        assert!(response.success);
        assert_eq!(response.message.as_deref(), Some("ok"));

        let response = send_ipc_command(
            &server_path,
            IpcCommand::Preview {
                path: String::new(),
                effect: None,
                duration_ms: None,
                no_theme: true,
                theme_override: None,
                monitor: None,
                scaling_mode: None,
            },
        )
        .await
        .unwrap();
        assert!(!response.success);
    }
}
