fn main() {
    guicons_build::IconBuild::auto()
        .emit(guicons_build::Emit::Rust)
        .emit(guicons_build::Emit::Slint)
        .build();

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let config = slint_build::CompilerConfiguration::new().with_include_paths(vec![out_dir.into()]);
    slint_build::compile_with_config("ui/main.slint", config).unwrap();
}
