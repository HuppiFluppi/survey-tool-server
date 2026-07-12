fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure().build_client(false).compile_protos(&["../../api/grpc/survey_tool.proto"], &["../../api/grpc/"])?;
    Ok(())
}
