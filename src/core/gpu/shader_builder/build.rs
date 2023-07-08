use spirv_builder::SpirvBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile for wgpu
    let path = SpirvBuilder::new("../stabilize_spirv", "spirv-unknown-vulkan1.2")
        //.print_metadata(spirv_builder::MetadataPrintout::Full).spirv_metadata(spirv_builder::SpirvMetadata::Full)
        .preserve_bindings(true)
        .build()?.module.unwrap_single().display().to_string();
    std::fs::rename(&path, format!("{}-f32", path)).unwrap();
    println!("cargo:rustc-env=stabilize_f32={}", format!("{}-f32", path));


    // Compile with u32 texture type
    std::env::set_var("RUSTGPU_RUSTFLAGS", "--cfg feature=\"texture_u32\"");
    let path = SpirvBuilder::new("../stabilize_spirv", "spirv-unknown-vulkan1.2")
        //.print_metadata(spirv_builder::MetadataPrintout::Full).spirv_metadata(spirv_builder::SpirvMetadata::Full)
        .preserve_bindings(true)
        .build()?.module.unwrap_single().display().to_string();
    std::fs::rename(&path, format!("{}-u32", path)).unwrap();
    println!("cargo:rustc-env=stabilize_u32={}", format!("{}-u32", path));


    // Compile for Qt RHI
    std::env::set_var("RUSTGPU_RUSTFLAGS", "--cfg feature=\"for_qtrhi\"");
    let path = SpirvBuilder::new("../stabilize_spirv", "spirv-unknown-vulkan1.2")
        //.print_metadata(spirv_builder::MetadataPrintout::Full).spirv_metadata(spirv_builder::SpirvMetadata::Full)
        .preserve_bindings(true)
        .build()?.module.unwrap_single().display().to_string();
    std::fs::rename(&path, format!("{}-rhi", path)).unwrap();
    println!("cargo:rustc-env=stabilize_qtrhi={}", format!("{}-rhi", path));


    Ok(())
}
