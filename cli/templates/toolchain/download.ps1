function Get-VerifiedArchive($row, $cache, $profile) {
    $directory = Join-Path $cache 'downloads'
    [void](New-Item $directory -ItemType Directory -Force)
    $target = Join-Path $directory "$($row.sha256)-$($row.filename)"
    $lock = [Threading.Mutex]::new($false, "Local\aio-toolchain-$($row.sha256)")
    $held = $false
    $temporary = "$target.part"
    try {
        try { $held = $lock.WaitOne(600000) } catch [Threading.AbandonedMutexException] { $held = $true }
        if (-not $held) { throw "等待工具下载锁超时: $($row.filename)" }
        if ((Test-Path $target) -and (Get-FileHash $target -Algorithm SHA256).Hash -eq $row.sha256) { return $target }
        if ($env:AIO_OFFLINE -eq '1') { throw "离线缓存缺失或校验失败: $($row.filename)。请联网运行一次，或预置 AIO_TOOLCHAIN_CACHE。" }
        $urls = @()
        if ($env:AIO_DOWNLOAD_ROOT) { $urls += "$($env:AIO_DOWNLOAD_ROOT.TrimEnd('/'))/$($row.filename)" }
        if ($profile -eq 'china') { $urls += @($row.mirror, $row.fallback) }
        $urls += $row.origin
        foreach ($url in $urls) {
            if (-not $url) { continue }
            [Console]::Error.WriteLine("下载工具: $($row.filename)")
            try {
                $downloaded = $false
                for ($attempt = 0; $attempt -lt 3; $attempt++) {
                    & curl.exe --fail --location --silent --show-error --continue-at - --connect-timeout 10 --max-time 300 --output $temporary $url
                    if ($LASTEXITCODE -eq 0) { $downloaded = $true; break }
                    if ($LASTEXITCODE -in @(33, 36)) { Remove-Item $temporary -Force -ErrorAction SilentlyContinue }
                    elseif ($LASTEXITCODE -notin @(5, 6, 7, 18, 28, 35, 52, 56)) { break }
                }
                if (-not $downloaded) { continue }
                if ((Get-FileHash $temporary -Algorithm SHA256).Hash -ne $row.sha256) {
                    [Console]::Error.WriteLine("工具摘要校验失败，尝试下一个源: $($row.filename)")
                    Remove-Item $temporary -Force
                    continue
                }
                Move-Item $temporary $target -Force
                return $target
            } catch { [Console]::Error.WriteLine("下载源不可用，尝试下一个源: $($row.filename)") }
        }
        throw "工具下载失败: $($row.filename)。可配置 HTTPS_PROXY 或 AIO_DOWNLOAD_ROOT，详见 NETWORK.md。"
    } finally {
        if ((Test-Path $temporary) -and (Get-Item $temporary).Length -eq 0) { Remove-Item $temporary -Force }
        if ($held) { $lock.ReleaseMutex() }
        $lock.Dispose()
    }
}

function Expand-VerifiedArchive($archive, $target, $required) {
    $stage = "$target.$([Guid]::NewGuid()).stage"
    [void](New-Item $stage -ItemType Directory -Force)
    $sha = (Get-FileHash $archive -Algorithm SHA256).Hash
    $lock = [Threading.Mutex]::new($false, "Local\aio-extract-$sha")
    $held = $false
    try {
        try { $held = $lock.WaitOne(600000) } catch [Threading.AbandonedMutexException] { $held = $true }
        if (-not $held) { throw '等待工具解压锁超时。' }
        if ((Test-Path "$target/.aio-sha256") -and (Get-Content "$target/.aio-sha256" -Raw).Trim() -eq $sha -and (Test-Path "$target/$required")) { return }
        & tar.exe -xf $archive -C $stage
        if ($LASTEXITCODE -ne 0) { throw '工具归档解压失败。' }
        if (-not (Test-Path "$stage/$required")) { throw '工具归档内容不完整。' }
        Set-Content "$stage/.aio-sha256" $sha -Encoding Ascii
        if (Test-Path $target) { Remove-Item $target -Recurse -Force }
        Move-Item $stage $target
    } finally {
        Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
        if ($held) { $lock.ReleaseMutex() }
        $lock.Dispose()
    }
}
