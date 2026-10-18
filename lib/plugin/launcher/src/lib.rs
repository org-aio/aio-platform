//! 固定启动器：在任何宿主代码之前运行，负责发布校验、原子切换与失败回滚。
//!
//! 启动器是宿主二进制自我演进的信任根，必须尽量小且稳定：只依赖文件系统、
//! 承载服务重启与 HTTP 健康入口。业务配置、数据库与插件都留给被它启动的宿主。
//!
//! 三个入口：
//! - `boot`：systemd 的 ExecStart。等待在途切换收敛、校验当前发布后 exec 成宿主，
//!   PID 与日志保持不变。
//! - `activate`：部署侧一次性执行。校验候选、原子切换、重启、健康确认，失败回滚。
//! - `repair`：把未确认的切换记录显式回滚到上一发布。

pub mod activate;
pub mod journal;
pub mod layout;
pub mod release;
pub mod supervisor;

mod cli;

pub use activate::{SwitchOutcome, SwitchRequest, activate, holder_running, repair, settle};
pub use journal::{Journal, Phase};
pub use layout::Layout;
pub use release::{RELEASE_MANIFEST, ReleaseManifest, safe_join};
pub use supervisor::{HostSupervisor, SystemdSupervisor};

/// 命令行入口；产品二进制直接转交这里，避免维护第二份参数解析。
pub use cli::run_cli;
