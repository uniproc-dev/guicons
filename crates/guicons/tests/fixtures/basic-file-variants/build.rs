use std::env;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    guicons::build::IconBuild::auto()
        .emit_rust_registry(out_dir.join("icons.rs"))
        .run();
}
