fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("protos/helloworld/helloworld.proto")
        .unwrap_or_else(|e| panic!("Failed to compile protos {:?}", e));
    tonic_build::compile_protos("protos/routeguide/routeguide.proto")
        .unwrap_or_else(|e| panic!("Failed to compile protos {:?}", e));
    Ok(())
}