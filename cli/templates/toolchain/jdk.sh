#!/bin/sh

aio_valid_jdk() {
    [ -n "$1" ] && [ -x "$1/bin/java" ] && [ -x "$1/bin/javac" ] && [ -f "$1/release" ] &&
        awk -F '"' '/^JAVA_VERSION=/{if ($2 ~ /^25([.+-]|$)/) found=1} END {exit !found}' "$1/release"
}

aio_find_jdk() (
    if [ -n "${AIO_JAVA_HOME:-}" ]; then
        if aio_valid_jdk "$AIO_JAVA_HOME"; then printf '%s\n' "$AIO_JAVA_HOME"; exit 0; fi
        echo "AIO_JAVA_HOME 必须指向完整的 JDK 25。" >&2
        exit 1
    fi
    if [ "${AIO_JDK_DOWNLOAD:-0}" != 1 ]; then
      if aio_valid_jdk "${JAVA_HOME:-}"; then printf '%s\n' "$JAVA_HOME"; exit 0; fi
      if [ -x /usr/libexec/java_home ]; then
        aio_mac_jdk=$(/usr/libexec/java_home -v 25 2>/dev/null || true)
        if aio_valid_jdk "$aio_mac_jdk"; then printf '%s\n' "$aio_mac_jdk"; exit 0; fi
      fi
      for aio_candidate in "$HOME"/Library/Java/JavaVirtualMachines/*/Contents/Home \
        /Library/Java/JavaVirtualMachines/*/Contents/Home "$HOME"/.jdks/* \
        "$HOME"/.sdkman/candidates/java/* /usr/lib/jvm/* /usr/java/* /opt/java/openjdk; do
        if aio_valid_jdk "$aio_candidate"; then printf '%s\n' "$aio_candidate"; exit 0; fi
      done
    fi
    aio_jdk_target="$AIO_TOOLCHAIN_CACHE/jdk-25.0.2-$aio_platform"
    case "$aio_platform" in darwin-*) aio_suffix=/jdk-25.0.2.jdk/Contents/Home ;; *) aio_suffix=/jdk-25.0.2 ;; esac
    if aio_valid_jdk "$aio_jdk_target$aio_suffix"; then printf '%s\n' "$aio_jdk_target$aio_suffix"; exit 0; fi
    mkdir -p "$AIO_TOOLCHAIN_CACHE"
    aio_acquire_lock "$aio_jdk_target.lock"
    if aio_valid_jdk "$aio_jdk_target$aio_suffix"; then printf '%s\n' "$aio_jdk_target$aio_suffix"; exit 0; fi
    aio_archive=$(aio_artifact jdk) || exit 1
    aio_stage=$(mktemp -d "$AIO_TOOLCHAIN_CACHE/jdk-stage.XXXXXX")
    trap 'rm -rf "$aio_stage"; rm -f "$aio_lock" "$aio_owner"' EXIT
    tar -xf "$aio_archive" -C "$aio_stage" || exit 1
    if ! aio_valid_jdk "$aio_stage$aio_suffix"; then echo "JDK 归档缺少有效的 Java 25。" >&2; exit 1; fi
    if [ -e "$aio_jdk_target" ]; then rm -rf "$aio_jdk_target"; fi
    mv "$aio_stage" "$aio_jdk_target" || exit 1
    printf '%s\n' "$aio_jdk_target$aio_suffix"
)
