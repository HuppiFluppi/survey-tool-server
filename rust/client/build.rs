//! Build script: compiles the shared gRPC proto into client stubs only
//! (server code generation is disabled).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure().build_server(false).compile_protos(&["../../api/grpc/survey_tool.proto"], &["../../api/grpc/"])?;
    Ok(())
}
