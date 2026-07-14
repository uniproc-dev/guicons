fn main() {
    guicons_build::IconBuild::auto()
        .emit(guicons_build::Emit::Rust)
        .build();
}
