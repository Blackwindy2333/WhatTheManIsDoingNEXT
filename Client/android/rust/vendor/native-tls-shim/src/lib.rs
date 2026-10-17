//! 编译替身：`native-tls` 0.2.x 的 API 面（仅 ureq 2.12.1 `src/ntls.rs` 用到的部分）。
//!
//! 为什么需要它：wtmd-core 的 ureq 依赖开启了 `native-tls` feature，而
//! native-tls 在 Android 上落地到 OpenSSL（openssl/openssl-sys），本机 Windows
//! 构建环境没有 perl/make，无法为 Android 交叉编译 OpenSSL，`cargo ndk` 会在
//! openssl-sys 的 build.rs 处失败。桥 workspace 用 `[patch.crates-io]` 把
//! native-tls 换成本 crate。
//!
//! 与真实 native-tls 的行为差异：TLS 握手在运行期直接返回错误。这与 ureq
//! 当前的实际运行期行为一致——ureq 仅开启 `native-tls`（未开 `tls`/rustls）
//! 时 `default_tls_config()` 返回 `NoTlsConfig`，https 请求本就不会经过
//! native_tls::TlsConnector（见 ureq 2.12.1 src/lib.rs 401-415 与
//! src/ntls.rs 的 `#[allow(dead_code)]`），http:// 上报完全不受影响。
//! 根治方案见 Client/android/README.md「偏差与取舍」。

use std::fmt;
use std::io::{Read, Write};

/// native_tls::Error 的最小替身。
#[derive(Debug, Clone)]
pub struct Error {
    message: String,
}

impl Error {
    pub fn new<S: Into<String>>(message: S) -> Error {
        Error {
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {}

/// native_tls::TlsStream<S> 的最小替身（持有底层流）。
pub struct TlsStream<S> {
    inner: S,
}

impl<S> TlsStream<S> {
    pub fn get_ref(&self) -> &S {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    pub fn into_inner(self) -> S {
        self.inner
    }
}

// Debug 无条件实现：避免 ureq 侧 `Box<dyn ReadWrite>` 的 Debug 边界问题。
impl<S> fmt::Debug for TlsStream<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsStream").finish_non_exhaustive()
    }
}

impl<S: Read + Write> Read for TlsStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<S: Read + Write> Write for TlsStream<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// native_tls::MidHandshakeTlsStream<S> 的最小替身（本替身永不构造）。
pub struct MidHandshakeTlsStream<S> {
    inner: S,
}

impl<S> MidHandshakeTlsStream<S> {
    pub fn get_ref(&self) -> &S {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S> fmt::Debug for MidHandshakeTlsStream<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidHandshakeTlsStream")
            .finish_non_exhaustive()
    }
}

/// native_tls::HandshakeError<S> 的最小替身。
#[derive(Debug)]
pub enum HandshakeError<S> {
    Failure(Error),
    WouldBlock(MidHandshakeTlsStream<S>),
}

impl<S> fmt::Display for HandshakeError<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandshakeError::Failure(e) => write!(f, "handshake failure: {}", e),
            HandshakeError::WouldBlock(_) => write!(f, "handshake would block"),
        }
    }
}

/// native_tls::TlsConnector 的最小替身。
#[derive(Debug, Clone)]
pub struct TlsConnector {
    _private: (),
}

impl TlsConnector {
    /// 与真实 native-tls 一样返回 Ok：ureq 的 ntls.rs 会对它 `.unwrap()`。
    pub fn new() -> Result<TlsConnector, Error> {
        Ok(TlsConnector { _private: () })
    }

    /// 握手必然失败：本替身不提供 TLS 后端（见文件头说明）。
    pub fn connect<S: Read + Write>(
        &self,
        _domain: &str,
        _stream: S,
    ) -> Result<TlsStream<S>, HandshakeError<S>> {
        Err(HandshakeError::Failure(Error::new(
            "native-tls shim: TLS backend unavailable in this build; \
             https reporting requires wiring a TLS connector in wtmd-core \
             (see Client/android/README.md)",
        )))
    }
}
