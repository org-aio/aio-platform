//! 切换日志：记录切换事务的持有者、目标与结果，供崩溃后判定是否需要回滚。
//!
//! 日志使用单文件原子替换，因此任一时刻磁盘上要么是完整旧记录，要么是完整新记录。

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

/// 切换阶段；只有 `Confirmed` 表示候选已经过健康检查。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 已切换到候选，尚未得到健康确认。
    Switching,
    /// 候选健康确认通过。
    Confirmed,
    /// 候选失败，已回滚到上一发布。
    RolledBack,
}

/// 在途切换的持有者标记。
///
/// 它把“activate 仍在运行”和“activate 已崩溃”区分开：`boot` 只在持有者进程
/// 已经消失、且心跳超过宽限期时才会接管回滚，避免与在途事务竞争。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Holder {
    /// 持有者进程号；同一台机器上的部署侧一次只允许一个切换。
    pub pid: u32,
    /// 持有者 CPU 起始时间（`/proc/<pid>/stat` 第 22 项），避免 PID 复用误判。
    pub started: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub revision: u64,
    pub phase: Phase,
    /// 当前期望运行的发布目录绝对路径。
    pub current: PathBuf,
    /// 上一成功发布；首次发布为空。
    pub previous: Option<PathBuf>,
    /// 在途切换的持有者；仅 `Switching` 期间存在。
    #[serde(default)]
    pub holder: Option<Holder>,
    /// 失败原因，仅在 `RolledBack` 时出现。
    pub detail: Option<String>,
}

impl Journal {
    /// 读取已有记录；文件不存在返回 `None`，损坏则报错而不是当作首次发布。
    pub async fn read(path: &Path) -> Result<Option<Self>> {
        let bytes = match tokio::fs::read(path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("读取切换日志失败: {}", path.display()));
            }
        };
        let journal: Self =
            serde_json::from_slice(&bytes).context("切换日志损坏，拒绝按首次启动处理")?;
        ensure!(journal.revision > 0, "切换日志序号必须大于零");
        Ok(Some(journal))
    }

    /// 通过同目录临时文件加 rename 原子写入，避免半写记录被后续进程误读。
    pub async fn write(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("切换日志缺少父目录")?;
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("创建日志目录失败: {}", parent.display()))?;
        let temporary = path.with_extension("tmp");
        let bytes = serde_json::to_vec_pretty(self).context("序列化切换日志失败")?;
        tokio::fs::write(&temporary, &bytes)
            .await
            .with_context(|| format!("写入临时日志失败: {}", temporary.display()))?;
        tokio::fs::rename(&temporary, path)
            .await
            .with_context(|| format!("提交切换日志失败: {}", path.display()))?;
        Ok(())
    }
}

/// 读取当前进程的持有者标记。
pub fn current_holder() -> Result<Holder> {
    let pid = std::process::id();
    Ok(Holder {
        pid,
        started: process_start_time(pid)?,
    })
}

/// 判断持有者是否仍在运行：PID 存在且起始时间一致才算存活。
pub fn holder_alive(holder: &Holder) -> bool {
    process_start_time(holder.pid).is_ok_and(|started| started == holder.started)
}

/// 读取 `/proc/<pid>/stat` 的进程起始时间；进程不存在或格式异常都返回错误。
fn process_start_time(pid: u32) -> Result<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .with_context(|| format!("读取进程 {pid} 状态失败"))?;
    // 进程名可能包含空格与括号，因此从最后一个 ')' 之后开始切分字段。
    let tail = stat
        .rsplit_once(')')
        .map(|(_, tail)| tail)
        .context("进程状态格式异常")?;
    let fields: Vec<&str> = tail.split_whitespace().collect();
    // tail 的第 1 项是 state，starttime 是第 20 项（stat 整体第 22 项）。
    let start = fields.get(19).context("进程状态缺少起始时间")?;
    start.parse().context("进程起始时间不是整数")
}
