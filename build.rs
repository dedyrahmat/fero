fn main() {
    println!("cargo:rustc-check-cfg=cfg(fero_strict_interface)");
    println!("cargo:rustc-check-cfg=cfg(fero_bound_dns)");
    println!("cargo:rustc-check-cfg=cfg(fero_routed_http)");
    println!("cargo:rerun-if-changed=src/bridge.rs");
    println!("cargo:rerun-if-changed=native/engine.h");
    println!("cargo:rerun-if-changed=native/engine.cc");

    let mut bridge = cxx_build::bridge("src/bridge.rs");
    bridge.file("native/engine.cc").include(".").std("c++17");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
            embed_windows_icon();
        }
        println!("cargo:rerun-if-env-changed=FERO_LIBTORRENT_PREFIX");
        println!("cargo:rerun-if-env-changed=FERO_NATIVE_DEPS");
        let prefix = std::path::PathBuf::from(
            std::env::var_os("FERO_LIBTORRENT_PREFIX")
                .expect("Set FERO_LIBTORRENT_PREFIX. See readme.md."),
        );
        let deps = std::path::PathBuf::from(
            std::env::var_os("FERO_NATIVE_DEPS").expect("Set FERO_NATIVE_DEPS. See readme.md."),
        );
        bridge
            .include(prefix.join("include"))
            .include(deps.join("include"));
        detect_strict_interface(&[prefix.join("include")]);
        watch_native_library(&[prefix.join("lib")]);
        bridge
            .define("_WIN32_WINNT", "0x0A00")
            .define("WIN32_LEAN_AND_MEAN", None)
            .define("TORRENT_LINKING_SHARED", None)
            .define("TORRENT_ABI_VERSION", "2")
            .define("TORRENT_USE_RTC", "0")
            .define("BOOST_ALL_NO_LIB", None)
            .define("BOOST_ASIO_ENABLE_CANCELIO", None)
            .define("BOOST_ASIO_NO_DEPRECATED", None)
            .define("BOOST_SYSTEM_USE_UTF8", None)
            .define("TORRENT_USE_OPENSSL", None)
            .define("TORRENT_USE_LIBCRYPTO", None)
            .define("TORRENT_SSL_PEERS", None);
        bridge.compile("fero-engine");
        println!(
            "cargo:rustc-link-search=native={}",
            prefix.join("lib").display()
        );
        println!("cargo:rustc-link-lib=dylib=torrent-rasterbar");
        println!("cargo:rustc-link-lib=dylib=ws2_32");
        return;
    }

    let probe = |metadata| {
        pkg_config::Config::new()
            .exactly_version("2.1.2")
            .cargo_metadata(metadata)
            .probe("libtorrent-rasterbar")
            .expect("Install libtorrent 2.1.2 and set PKG_CONFIG_PATH. See readme.md.")
    };
    let native = probe(false);
    detect_strict_interface(&native.include_paths);
    watch_native_library(&native.link_paths);
    bridge.includes(native.include_paths);
    for (name, value) in &native.defines {
        bridge.define(name, value.as_deref());
    }
    bridge.compile("fero-engine");
    probe(true);
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=framework=SystemConfiguration");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
    }
}

fn embed_windows_icon() {
    println!("cargo:rerun-if-changed=native/windows/fero.rc");
    println!("cargo:rerun-if-changed=native/windows/Fero.ico");
    let resource = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("fero.res");
    let status = std::process::Command::new("rc.exe")
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource)
        .arg("native/windows/fero.rc")
        .status()
        .expect("Run Cargo from an x64 Visual Studio Developer PowerShell with the Windows SDK");
    assert!(status.success(), "Could not compile Fero's Windows icon");
    println!("cargo:rustc-link-arg-bin=fero={}", resource.display());
}

fn detect_strict_interface(include_paths: &[std::path::PathBuf]) {
    for directory in include_paths {
        if directory.join("libtorrent/fero_http.hpp").is_file() {
            println!("cargo:rustc-cfg=fero_routed_http");
        }
        let settings = directory.join("libtorrent/settings_pack.hpp");
        if let Ok(header) = std::fs::read_to_string(&settings) {
            println!("cargo:rerun-if-changed={}", settings.display());
            if header.contains("#define FERO_LIBTORRENT_STRICT_INTERFACE 1") {
                println!("cargo:rustc-cfg=fero_strict_interface");
            }
            if header.contains("#define FERO_LIBTORRENT_BOUND_DNS 1") {
                println!("cargo:rustc-cfg=fero_bound_dns");
            }
            break;
        }
    }
}

fn watch_native_library(link_paths: &[std::path::PathBuf]) {
    for path in link_paths {
        for name in [
            "libtorrent-rasterbar.a",
            "libtorrent-rasterbar.so",
            "libtorrent-rasterbar.dylib",
            "torrent-rasterbar.lib",
        ] {
            let library = path.join(name);
            if library.is_file() {
                println!("cargo:rerun-if-changed={}", library.display());
            }
        }
    }
}
