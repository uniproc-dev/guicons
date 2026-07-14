use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use guicons_build::{Emit, IconBuild};
use std::fs;
use std::path::{Path, PathBuf};

/// Writes a manifest with `n` families, each with a `regular`/`filled` file
/// variant, to exercise the same builder codegen a downstream crate's
/// build.rs would run.
fn write_manifest(dir: &Path, n: usize) -> PathBuf {
    let assets_dir = dir.join("assets");
    fs::create_dir_all(&assets_dir).unwrap();

    let mut manifest = String::from("[defaults]\nroot = \"assets\"\n\n");
    for i in 0..n {
        let regular = format!("icon-{i}-regular.svg");
        let filled = format!("icon-{i}-filled.svg");
        fs::write(assets_dir.join(&regular), b"<svg></svg>").unwrap();
        fs::write(assets_dir.join(&filled), b"<svg></svg>").unwrap();
        manifest.push_str(&format!(
            "[icon-{i}]\nvariants.regular = {{ file = \"{regular}\" }}\nvariants.filled = {{ file = \"{filled}\" }}\n\n"
        ));
    }

    let manifest_path = dir.join("icons.gui.toml");
    fs::write(&manifest_path, manifest).unwrap();
    manifest_path
}

fn bench_codegen(c: &mut Criterion) {
    let mut group = c.benchmark_group("codegen");
    for n in [10usize, 100, 500] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let dir = tempfile::tempdir().unwrap();
                    let manifest_path = write_manifest(dir.path(), n);
                    let out_dir = dir.path().join("out");
                    fs::create_dir_all(&out_dir).unwrap();
                    (dir, manifest_path, out_dir)
                },
                |(dir, manifest_path, out_dir)| {
                    unsafe { std::env::set_var("OUT_DIR", &out_dir) };
                    IconBuild::new(manifest_path)
                        .materialized_root(out_dir)
                        .emit(Emit::Rust)
                        .emit(Emit::Slint)
                        .build();
                    drop(dir);
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_codegen);
criterion_main!(benches);
