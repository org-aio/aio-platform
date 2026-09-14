param([string]$Wrapper, [Parameter(ValueFromRemainingArguments = $true)][string[]]$BuildArguments)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
. "$PSScriptRoot/download.ps1"
try {
    $profile = if ($env:AIO_NETWORK) { $env:AIO_NETWORK } else { (Get-Content "$PSScriptRoot/profile" -Raw).Trim() }
    if ($profile -notin @('china', 'global')) { throw 'AIO_NETWORK 只支持 china 或 global。' }
    $architecture = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }
    $platform = "win32-$architecture"
    $cache = if ($env:AIO_TOOLCHAIN_CACHE) { $env:AIO_TOOLCHAIN_CACHE } else { Join-Path $env:LOCALAPPDATA 'aio/toolchains' }
    $artifacts = Import-Csv "$PSScriptRoot/artifacts.tsv" -Delimiter "`t"
    function Get-Artifact($kind) {
        $row = $artifacts | Where-Object { $_.kind -eq $kind -and ($_.platform -eq $platform -or $_.platform -eq 'all') } | Select-Object -First 1
        if (-not $row) { throw "工具链暂不支持 $platform 的 $kind 自动下载，详见 NETWORK.md。" }
        return $row
    }
    function Test-Jdk($path) {
        return $path -and (Test-Path "$path/bin/javac.exe") -and (Test-Path "$path/bin/java.exe") -and
            (Test-Path "$path/release") -and ((Get-Content "$path/release" -Raw) -match '(?m)^JAVA_VERSION="25([.+-]|")')
    }
    $jdk = $null
    if ($env:AIO_JAVA_HOME) {
        if (-not (Test-Jdk $env:AIO_JAVA_HOME)) { throw 'AIO_JAVA_HOME 必须指向完整的 JDK 25。' }
        $jdk = $env:AIO_JAVA_HOME
    } elseif ($env:AIO_JDK_DOWNLOAD -ne '1' -and (Test-Jdk $env:JAVA_HOME)) { $jdk = $env:JAVA_HOME }
    if (-not $jdk -and $env:AIO_JDK_DOWNLOAD -ne '1') {
        $candidates = @("$env:USERPROFILE/.jdks/*", "$env:ProgramFiles/Java/*", "$env:ProgramFiles/Eclipse Adoptium/*", "$env:ProgramFiles/Microsoft/jdk-*")
        foreach ($pattern in $candidates) {
            foreach ($directory in (Get-ChildItem $pattern -Directory -ErrorAction SilentlyContinue)) {
                if (Test-Jdk $directory.FullName) { $jdk = $directory.FullName; break }
            }
            if ($jdk) { break }
        }
    }
    if (-not $jdk) {
        $row = Get-Artifact 'jdk'
        $target = Join-Path $cache "jdk-25.0.2-$platform"
        if (-not (Test-Jdk "$target/jdk-25.0.2")) {
            $archive = Get-VerifiedArchive $row $cache $profile
            Expand-VerifiedArchive $archive $target 'jdk-25.0.2/bin/javac.exe'
        }
        $jdk = "$target/jdk-25.0.2"
        if (-not (Test-Jdk $jdk)) { throw 'JDK 归档缺少有效的 Java 25。' }
    }
    $env:JAVA_HOME = $jdk
    if (-not $env:KOTLIN_CLI_JAVA_HOME) { $env:KOTLIN_CLI_JAVA_HOME = $jdk }
    if (-not $env:KOTLIN_SHARED_CACHE_DIR) { $env:KOTLIN_SHARED_CACHE_DIR = Join-Path $env:LOCALAPPDATA 'JetBrains/Kotlin' }
    if ($profile -eq 'china' -and -not $env:npm_config_registry) { $env:npm_config_registry = 'https://registry.npmmirror.com' }
    if ((Get-Content "$PSScriptRoot/web" -Raw).Trim() -eq 'true' -and ($BuildArguments | Where-Object { $_ -in @('build', 'run', 'task') })) {
        $shared = $env:KOTLIN_SHARED_CACHE_DIR
        for ($i = 0; $i -lt $BuildArguments.Count; $i++) {
            if ($BuildArguments[$i] -eq '--shared-cache-dir' -and $i + 1 -lt $BuildArguments.Count) { $shared = $BuildArguments[++$i] }
            elseif ($BuildArguments[$i].StartsWith('--shared-cache-dir=')) { $shared = $BuildArguments[$i].Substring(19) }
        }
        [void](New-Item "$shared/download.cache" -ItemType Directory -Force)
        foreach ($kind in @('node', 'pnpm')) {
            $row = Get-Artifact $kind
            $archive = Get-VerifiedArchive $row $cache $profile
            $hasher = [Security.Cryptography.SHA256]::Create()
            try { $key = ([BitConverter]::ToString($hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($row.origin + 'V1')))).Replace('-', '').Substring(0, 10).ToLowerInvariant() }
            finally { $hasher.Dispose() }
            $destination = Join-Path "$shared/download.cache" "$key-$($row.filename)"
            if ((Test-Path $destination) -and (Get-FileHash $destination -Algorithm SHA256).Hash -eq $row.sha256) { continue }
            $temporary = "$destination.$([Guid]::NewGuid()).part"
            try { Copy-Item $archive $temporary; Move-Item $temporary $destination -Force }
            finally { Remove-Item $temporary -Force -ErrorAction SilentlyContinue }
        }
    }
    $cli = Get-Artifact 'cli'
    if ($env:KOTLIN_CLI_DOWNLOAD_ROOT) {
        $cli.origin = "$($env:KOTLIN_CLI_DOWNLOAD_ROOT.TrimEnd('/'))/org/jetbrains/kotlin/kotlin-cli/0.12.0-dev-4233/$($cli.filename)"
    }
    if (-not $env:KOTLIN_CLI_BOOTSTRAP_CACHE_DIR) { $env:KOTLIN_CLI_BOOTSTRAP_CACHE_DIR = Join-Path $env:LOCALAPPDATA 'JetBrains/Kotlin/cli' }
    $cliTarget = Join-Path $env:KOTLIN_CLI_BOOTSTRAP_CACHE_DIR 'kotlin-cli-0.12.0-dev-4233'
    if (-not (Test-Path "$cliTarget/.flag") -or (Get-Content "$cliTarget/.flag" -Raw).Trim() -ne $cli.sha256) {
        $archive = Get-VerifiedArchive $cli $cache $profile
        Expand-VerifiedArchive $archive $cliTarget 'bin/launcher.sh'
        Set-Content "$cliTarget/.flag" $cli.sha256 -Encoding Ascii
    }
    $env:AIO_TOOLCHAIN_READY = '1'
    & $Wrapper @BuildArguments
    exit $LASTEXITCODE
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
