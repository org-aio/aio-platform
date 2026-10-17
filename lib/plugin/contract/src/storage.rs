//! 各执行器与原生 SDK 共用的对象存储边界。

/// 单个对象最大字节数，上传、下载和后端句柄均必须执行此限制。
pub const MAX_OBJECT_BYTES: usize = 16 * 1024 * 1024;
/// 对象键使用 UTF-8 字节计数；不作为文件系统路径解释。
pub const MAX_OBJECT_KEY_BYTES: usize = 1024;
/// Sidecar broker 请求包含读取请求体和后端访问的总截止时间。
pub const OBJECT_TIMEOUT_SECONDS: u64 = 30;
