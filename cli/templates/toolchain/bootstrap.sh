#!/bin/sh

aio_tools=$(CDPATH= cd -- "$(dirname -- "$0")/.aio/toolchain" && pwd)
AIO_NETWORK=${AIO_NETWORK:-$(cat "$aio_tools/profile")}
case "$AIO_NETWORK" in china|global) ;; *) echo "AIO_NETWORK 只支持 china 或 global。" >&2; exit 1 ;; esac
case "$(uname -s)" in
    Darwin) aio_os=darwin; aio_cache="$HOME/Library/Caches" ;;
    Linux) aio_os=linux; aio_cache="${XDG_CACHE_HOME:-$HOME/.cache}" ;;
    *) echo "请在 Windows 使用 kotlin.bat；当前系统不支持此工具链。" >&2; exit 1 ;;
esac
case "$(uname -m)" in
    aarch64|arm64) aio_arch=arm64 ;;
    x86_64|amd64) aio_arch=x64 ;;
    *) echo "工具链暂不支持当前 CPU 架构。" >&2; exit 1 ;;
esac
aio_platform="$aio_os-$aio_arch"
AIO_TOOLCHAIN_CACHE=${AIO_TOOLCHAIN_CACHE:-$aio_cache/aio/toolchains}
KOTLIN_SHARED_CACHE_DIR=${KOTLIN_SHARED_CACHE_DIR:-$aio_cache/JetBrains/Kotlin}
. "$aio_tools/download.sh"
. "$aio_tools/jdk.sh"
JAVA_HOME=$(aio_find_jdk) || exit 1
KOTLIN_CLI_JAVA_HOME=${KOTLIN_CLI_JAVA_HOME:-$JAVA_HOME}
export JAVA_HOME KOTLIN_CLI_JAVA_HOME KOTLIN_SHARED_CACHE_DIR
if [ "$AIO_NETWORK" = china ]; then
    npm_config_registry=${npm_config_registry:-https://registry.npmmirror.com}
    export npm_config_registry
fi

# 上游工具地址固定，以它的缓存键预置原始归档；升级 CLI 时同步锁文件。
aio_prepare_web() (
    aio_shared=$KOTLIN_SHARED_CACHE_DIR
    aio_next=false
    for aio_arg in "$@"; do
        if [ "$aio_next" = true ]; then aio_shared=$aio_arg; aio_next=false; fi
        case "$aio_arg" in
            --shared-cache-dir) aio_next=true ;;
            --shared-cache-dir=*) aio_shared=${aio_arg#*=} ;;
        esac
    done
    mkdir -p "$aio_shared/download.cache"
    for aio_kind in node pnpm; do
        aio_archive=$(aio_artifact "$aio_kind") || exit 1
        aio_record=$(awk -F '\t' -v kind="$aio_kind" -v platform="$aio_platform" '$1 == kind && $2 == platform {print; exit}' "$aio_tools/artifacts.tsv")
        aio_origin=$(printf '%s\n' "$aio_record" | cut -f5)
        aio_filename=$(printf '%s\n' "$aio_record" | cut -f3)
        aio_key_file=$(mktemp "$AIO_TOOLCHAIN_CACHE/cache-key.XXXXXX")
        printf '%sV1' "$aio_origin" > "$aio_key_file"
        aio_key=$(aio_sha256 "$aio_key_file" | cut -c1-10)
        rm -f "$aio_key_file"
        aio_destination="$aio_shared/download.cache/$aio_key-$aio_filename"
        if [ -f "$aio_destination" ] && cmp -s "$aio_archive" "$aio_destination"; then continue; fi
        aio_copy=$(mktemp "$aio_shared/download.cache/aio-copy.XXXXXX")
        if ! cp "$aio_archive" "$aio_copy" || ! mv -f "$aio_copy" "$aio_destination"; then
            rm -f "$aio_copy"
            exit 1
        fi
    done
)
if [ "$(cat "$aio_tools/web")" = true ]; then
    case " $* " in
        *' build '*|*' run '*|*' task '*) aio_prepare_web "$@" || exit 1 ;;
    esac
fi
