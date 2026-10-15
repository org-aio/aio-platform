pub(crate) fn permitted(name: &str) -> bool {
    let Some((interface, version)) = name.rsplit_once('@') else {
        return false;
    };
    if version == "2.0.0"
        && matches!(
            interface,
            "aio:plugin/host"
                | "aio:plugin/database"
                | "aio:plugin/storage"
                | "aio:plugin/management"
                | "aio:plugin/enhancements"
                | "aio:plugin/cryptography"
                | "aio:plugin/transport"
                | "aio:plugin/metadata"
        )
    {
        return true;
    }
    let Ok(version) = semver::Version::parse(version) else {
        return false;
    };
    // 仅接受当前 Wasmtime 支持的稳定 WASI 补丁版本；网络和文件系统仍不开放。
    version.major == 0
        && version.minor == 2
        && (6..=12).contains(&version.patch)
        && version.pre.is_empty()
        && version.build.is_empty()
        && matches!(
            interface,
            "wasi:io/error"
                | "wasi:io/streams"
                | "wasi:io/poll"
                | "wasi:clocks/monotonic-clock"
                | "wasi:clocks/wall-clock"
                | "wasi:cli/stdin"
                | "wasi:cli/stdout"
                | "wasi:cli/stderr"
                | "wasi:cli/environment"
                | "wasi:cli/exit"
                | "wasi:cli/terminal-input"
                | "wasi:cli/terminal-output"
                | "wasi:cli/terminal-stdin"
                | "wasi:cli/terminal-stdout"
                | "wasi:cli/terminal-stderr"
                | "wasi:random/random"
                | "wasi:random/insecure-seed"
        )
}

#[cfg(test)]
mod tests {
    use super::permitted;

    #[test]
    fn limits_import_interfaces_and_versions() {
        for name in [
            "aio:plugin/host@2.0.0",
            "wasi:io/streams@0.2.6",
            "wasi:cli/environment@0.2.9",
            "wasi:random/insecure-seed@0.2.9",
        ] {
            assert!(permitted(name));
        }
        for name in [
            "aio:plugin/host@1.0.0",
            "wasi:filesystem/types@0.2.9",
            "wasi:sockets/tcp@0.2.9",
            "wasi:cli/environment@0.3.0",
            "wasi:io/poll@0.2.13",
            "wasi:io/poll@0.2.9-dev",
            "wasi:io/poll@0.2.9+custom",
            "wasi:io/poll",
            "wasi:unknown/resource@0.2.9",
            "wasi:random/insecure-seed@0.2.13",
            "wasi:random/insecure-seed@0.3.0",
        ] {
            assert!(!permitted(name), "{name}");
        }
    }
}
