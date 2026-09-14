#!/bin/sh

aio_sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

aio_transfer() {
    aio_attempt=0
    while [ "$aio_attempt" -lt 3 ]; do
        aio_attempt=$((aio_attempt + 1))
        if command -v curl >/dev/null 2>&1; then
            aio_code=0
            curl --fail --location --silent --show-error --continue-at - --connect-timeout 10 \
                --max-time 300 --output "$2" "$1" || aio_code=$?
            case "$aio_code" in
                0) return 0 ;;
                33|36) rm -f "$2" ;;
                5|6|7|18|28|35|52|56) ;;
                *) return 1 ;;
            esac
        elif command -v wget >/dev/null 2>&1; then
            wget -q -c --tries=1 --timeout=60 -O "$2" "$1" && return 0
        else
            echo "请安装 curl 或 wget 后重试。" >&2
            return 1
        fi
    done
    return 1
}

aio_acquire_lock() {
    aio_lock=$1
    aio_owner="$aio_lock.$$.owner"
    printf '%s\n' "$$" > "$aio_owner"
    aio_wait=0
    trap 'rm -f "$aio_owner"' EXIT
    while ! ln "$aio_owner" "$aio_lock" 2>/dev/null; do
        aio_pid=$(cat "$aio_lock" 2>/dev/null || true)
        if [ -n "$aio_pid" ] && ! kill -0 "$aio_pid" 2>/dev/null; then
            if [ "$(cat "$aio_lock" 2>/dev/null)" = "$aio_pid" ]; then rm -f "$aio_lock"; fi
            continue
        fi
        aio_wait=$((aio_wait + 1))
        if [ "$aio_wait" -ge 600 ]; then echo "等待工具缓存锁超时。" >&2; exit 1; fi
        sleep 1
    done
    trap 'rm -f "$aio_lock" "$aio_owner"' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
}

# 每个归档独立加锁；只发布经过摘要校验的完整文件。
aio_download() (
    aio_file=$1 aio_sha=$2 aio_origin=$3
    shift 3
    mkdir -p "$AIO_TOOLCHAIN_CACHE/downloads"
    aio_target="$AIO_TOOLCHAIN_CACHE/downloads/$aio_sha-$aio_file"
    if [ -f "$aio_target" ] && [ "$(aio_sha256 "$aio_target")" = "$aio_sha" ]; then
        printf '%s\n' "$aio_target"
        exit 0
    fi
    if [ "${AIO_OFFLINE:-0}" = 1 ]; then
        echo "离线缓存缺失或校验失败: ${aio_file}。请联网运行一次，或预置 AIO_TOOLCHAIN_CACHE。" >&2
        exit 1
    fi
    aio_acquire_lock "$aio_target.lock"
    aio_temp="$aio_target.part"
    if [ -f "$aio_target" ] && [ "$(aio_sha256 "$aio_target")" = "$aio_sha" ]; then
        printf '%s\n' "$aio_target"
        exit 0
    fi
    if [ "$AIO_NETWORK" = global ]; then set --; fi
    set -- "$@" "$aio_origin"
    if [ -n "${AIO_DOWNLOAD_ROOT:-}" ]; then set -- "${AIO_DOWNLOAD_ROOT%/}/$aio_file" "$@"; fi
    for aio_url in "$@"; do
        [ -n "$aio_url" ] || continue
        echo "下载工具: $aio_file" >&2
        aio_transfer "$aio_url" "$aio_temp" || continue
        if [ "$(aio_sha256 "$aio_temp")" != "$aio_sha" ]; then
            echo "工具摘要校验失败，尝试下一个源: $aio_file" >&2
            rm -f "$aio_temp"
            continue
        fi
        mv -f "$aio_temp" "$aio_target"
        printf '%s\n' "$aio_target"
        exit 0
    done
    if [ -f "$aio_temp" ] && [ ! -s "$aio_temp" ]; then rm -f "$aio_temp"; fi
    echo "工具下载失败: ${aio_file}。可重试续传，或配置 HTTPS_PROXY / AIO_DOWNLOAD_ROOT，详见 NETWORK.md。" >&2
    exit 1
)

aio_artifact() {
    aio_record=$(awk -F '\t' -v kind="$1" -v platform="$aio_platform" '$1 == kind && $2 == platform {print; exit}' "$aio_tools/artifacts.tsv")
    if [ -z "$aio_record" ]; then
        echo "工具链暂不支持 $aio_platform 的 $1 自动下载，详见 NETWORK.md。" >&2
        return 1
    fi
    aio_filename=$(printf '%s\n' "$aio_record" | cut -f3)
    aio_digest=$(printf '%s\n' "$aio_record" | cut -f4)
    aio_origin=$(printf '%s\n' "$aio_record" | cut -f5)
    aio_mirror=$(printf '%s\n' "$aio_record" | cut -f6)
    aio_fallback=$(printf '%s\n' "$aio_record" | cut -f7)
    aio_download "$aio_filename" "$aio_digest" "$aio_origin" "$aio_mirror" "$aio_fallback"
}
