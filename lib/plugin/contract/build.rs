fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "grpc")]
    {
        // 固定工具来源，通过配置传入 protoc，避免修改并行构建的进程环境。
        let mut config = tonic_prost_build::Config::new();
        config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
        tonic_prost_build::configure().compile_with_config(
            config,
            &["proto/enhancements.proto"],
            &["proto"],
        )?;
    }
    Ok(())
}
