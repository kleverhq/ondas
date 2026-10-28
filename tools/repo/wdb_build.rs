use std::{env, path::PathBuf, process::Command};

pub(crate) fn build() {
    println!("cargo:rerun-if-env-changed=XILINX_VIVADO");
    for source in ["native/wdb.cpp", "native/wdb.h", "native/wdb_abi.hpp"] {
        println!("cargo:rerun-if-changed={source}");
    }
    assert_eq!(
        env::var("TARGET").unwrap(),
        "x86_64-unknown-linux-gnu",
        "wdb-lib requires x86_64-unknown-linux-gnu"
    );
    let home = PathBuf::from(
        env::var_os("XILINX_VIVADO")
            .expect("wdb-lib requires XILINX_VIVADO pointing to an installed reader"),
    )
    .canonicalize()
    .expect("wdb-lib: inaccessible XILINX_VIVADO");
    let lib = home.join("lib/lnx64.o");
    let mut build = cc::Build::new();
    let data = home
        .join("data")
        .canonicalize()
        .expect("wdb-lib: missing reader data directory");
    let data = data.to_str().expect("wdb-lib: data path must be UTF-8");
    assert!(
        !data.contains(['\n', '\r']),
        "wdb-lib: newline in data path"
    );
    let data = format!("\"{}\"", data.replace('\\', "\\\\").replace('"', "\\\""));
    build.define("ONDAS_WDB_DATA", data.as_str());
    for (name, define, required) in [
        (
            "libxv_wavedata.so",
            "ONDAS_WDB_READER",
            "_ZN9SimBridge10DataReader13getDataObjMgrEv",
        ),
        (
            "libxv_simbridge_kernel.so",
            "ONDAS_WDB_KERNEL",
            "_ZNK9SimBridge15WdbCommonReader13getValueEventExRKSt6vectorIPNS_14WdbScalarBTreeESaIS3_EERNS_17WdbTimeByteValuesE",
        ),
    ] {
        let path = lib
            .join(name)
            .canonicalize()
            .unwrap_or_else(|error| panic!("wdb-lib: {}: {error}", lib.join(name).display()));
        println!("cargo:rerun-if-changed={}", path.display());
        let path_text = path.to_str().expect("wdb-lib: library path must be UTF-8");
        assert!(
            !path_text.contains(['\n', '\r']),
            "wdb-lib: newline in library path"
        );
        let quoted = format!(
            "\"{}\"",
            path_text.replace('\\', "\\\\").replace('"', "\\\"")
        );
        build.define(define, quoted.as_str());
        let symbols = Command::new("nm")
            .args(["-D", "--defined-only"])
            .arg(&path)
            .output()
            .expect("wdb-lib: nm (binutils) is required");
        assert!(symbols.status.success(), "wdb-lib: cannot inspect reader");
        let listing = String::from_utf8_lossy(&symbols.stdout);
        if name == "libxv_wavedata.so" {
            let dependencies = Command::new("readelf")
                .args(["-d"])
                .arg(&path)
                .env("LC_ALL", "C")
                .output()
                .expect("wdb-lib: readelf (binutils) is required");
            assert!(
                dependencies.status.success(),
                "wdb-lib: cannot inspect reader dependencies"
            );
            assert!(
                String::from_utf8_lossy(&dependencies.stdout)
                    .lines()
                    .any(|line| {
                        line.contains("(NEEDED)") && line.contains("[libxv_simbridge_kernel.so]")
                    }),
                "wdb-lib: reader must load its selected kernel dependency"
            );
        }
        assert!(
            listing
                .lines()
                .any(|line| line.split_whitespace().last() == Some(required)),
            "wdb-lib: reader is missing required ABI symbol {required}"
        );
    }
    build
        .cpp(true)
        .std("c++17")
        .file("native/wdb.cpp")
        .compile("ondas_wdb_shim");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=xv_wavedata");
    println!("cargo:rustc-link-lib=dl");
}
