fn main() {
    println!("cargo:rerun-if-changed=ui/osk.slint");
    println!("cargo:rerun-if-changed=ui/fonts/DejaVuSans.ttf");
    println!("cargo:rerun-if-changed=ui/fonts/DejaVuSans-Bold.ttf");

    if std::env::var_os("CARGO_FEATURE_FUTO_SWIPE").is_some() {
        link_futo_runtime();
    } else {
        build_geometric_swipe();
    }

    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![std::path::PathBuf::from("ui/fonts")])
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config("ui/osk.slint", config).unwrap();
}

fn build_geometric_swipe() {
    let sources = [
        "third_party/openswipe/engine/src/swipe_layout.cpp",
        "third_party/openswipe/engine/src/swipe_net.cpp",
        "third_party/openswipe/engine/src/swipe_beam.cpp",
        "third_party/openswipe/engine/src/swipe_geo.cpp",
        "third_party/openswipe/engine/src/swipe_engine.cpp",
        "third_party/openswipe/shim.cpp",
    ];
    for source in sources {
        println!("cargo:rerun-if-changed={source}");
    }
    let headers = [
        "third_party/openswipe/engine/include/swipekb/swipe_beam.hpp",
        "third_party/openswipe/engine/include/swipekb/swipe_engine.hpp",
        "third_party/openswipe/engine/include/swipekb/swipe_geo.hpp",
        "third_party/openswipe/engine/include/swipekb/swipe_layout.hpp",
        "third_party/openswipe/engine/include/swipekb/swipe_net.hpp",
    ];
    for header in headers {
        println!("cargo:rerun-if-changed={header}");
    }
    cc::Build::new()
        .cpp(true)
        .std("c++20")
        .include("third_party/openswipe/engine/include")
        .files(sources)
        .warnings(false)
        .flag_if_supported("-static-libstdc++")
        .flag_if_supported("-static-libgcc")
        .compile("hyprosk-swipekb");
}

fn link_futo_runtime() {
    println!("cargo:rerun-if-env-changed=EXECUTORCH_RS_EXECUTORCH_LIB_DIR");
    let root = std::env::var_os("EXECUTORCH_RS_EXECUTORCH_LIB_DIR")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_dir())
        .expect("futo-swipe needs EXECUTORCH_RS_EXECUTORCH_LIB_DIR pointing to an ExecuTorch 1.5.1 CMake build");
    let paths = [
        root.join("kernels/portable"),
        root.join("backends/xnnpack"),
        root.join("extension/threadpool"),
        root.join("backends/xnnpack/third-party/XNNPACK"),
        root.join("backends/xnnpack/third-party/cpuinfo"),
        root.join("backends/xnnpack/third-party/pthreadpool"),
    ];
    for path in paths {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
    println!("cargo:rustc-link-lib=static:+whole-archive=portable_ops_lib");
    println!("cargo:rustc-link-lib=static:+whole-archive=xnnpack_backend");
    println!("cargo:rustc-link-lib=static:+whole-archive=portable_kernels");
    for library in [
        "extension_threadpool",
        "XNNPACK",
        "xnnpack-microkernels-prod",
        "cpuinfo",
        "pthreadpool",
    ] {
        println!("cargo:rustc-link-lib=static:+whole-archive={library}");
    }
}
