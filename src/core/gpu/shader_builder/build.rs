use spirv_builder::SpirvBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile for wgpu
    SpirvBuilder::new("../stabilize_spirv", "spirv-unknown-vulkan1.2")
        //.print_metadata(spirv_builder::MetadataPrintout::Full)
        //.spirv_metadata(spirv_builder::SpirvMetadata::Full)
        .preserve_bindings(true)
        .build()?;

    // Compile for Qt RHI
    std::env::set_var("RUSTGPU_RUSTFLAGS", "--cfg feature=\"for_glsl\"");
    SpirvBuilder::new("../stabilize_spirv", "spirv-unknown-vulkan1.1spv1.4")
        //.print_metadata(spirv_builder::MetadataPrintout::Full)
        //.spirv_metadata(spirv_builder::SpirvMetadata::Full)
        .preserve_bindings(true)
        .build()?;
    Ok(())
}
