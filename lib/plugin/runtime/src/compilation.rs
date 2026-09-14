use std::path::Path;

use anyhow::{Context, Result};
use wasmtime::{Cache, CacheConfig};

/// 仅由宿主写入的编译缓存；引擎版本、目标架构和编译配置由 Wasmtime 纳入缓存键。
pub fn compilation_cache(directory: &Path) -> Result<Cache> {
    let mut config = CacheConfig::new();
    config
        .with_directory(directory)
        .with_files_total_size_soft_limit(512 * 1024 * 1024)
        .with_file_count_soft_limit(4096);
    Cache::new(config)
        .map_err(anyhow::Error::from)
        .context("创建插件编译缓存失败")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::{Config, Engine, Module};

    #[test]
    fn a_new_engine_reuses_disk_code_and_rejects_changed_configuration() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let wasm = [
            0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 127, 3, 2, 1, 0, 7, 5, 1, 1, 102, 0, 0,
            10, 6, 1, 4, 0, 65, 42, 11,
        ];
        let compile = |fuel| -> Result<(usize, usize)> {
            let cache = compilation_cache(directory.path())?;
            let mut config = Config::new();
            config.cache(Some(cache.clone())).consume_fuel(fuel);
            let engine = Engine::new(&config)?;
            Module::new(&engine, wasm)?;
            Ok((cache.cache_hits(), cache.cache_misses()))
        };
        assert_eq!(compile(false)?, (0, 1));
        assert_eq!(compile(false)?, (1, 0));
        assert_eq!(compile(true)?, (0, 1));
        Ok(())
    }
}
