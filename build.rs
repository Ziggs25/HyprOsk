fn main() {
    println!("cargo:rerun-if-changed=ui/osk.slint");
    println!("cargo:rerun-if-changed=ui/fonts/DejaVuSans.ttf");
    println!("cargo:rerun-if-changed=ui/fonts/DejaVuSans-Bold.ttf");
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![std::path::PathBuf::from("ui/fonts")])
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config("ui/osk.slint", config).unwrap();
}