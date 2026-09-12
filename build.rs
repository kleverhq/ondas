fn main() {
    #[cfg(feature = "fsdb-lib")]
    fsdb();
}

#[cfg(feature = "fsdb-lib")]
fn fsdb() {
    use std::{env, fs, path::PathBuf, process::Command};

    println!("cargo:rerun-if-env-changed=VERDI_HOME");
    println!("cargo:rerun-if-changed=native/fsdb.cpp");
    println!("cargo:rerun-if-changed=native/fsdb.h");
    println!("cargo:rerun-if-changed=native/fsdb_deps.S");
    assert_eq!(
        env::var("TARGET").unwrap(),
        "x86_64-unknown-linux-gnu",
        "fsdb-lib requires x86_64-unknown-linux-gnu"
    );
    let home = PathBuf::from(
        env::var_os("VERDI_HOME")
            .expect("fsdb-lib requires VERDI_HOME pointing to an installed Verdi FSDB Reader SDK"),
    )
    .canonicalize()
    .expect("fsdb-lib: VERDI_HOME is not an accessible directory");
    let sdk = home.join("share/FsdbReader");
    for header in ["ffrAPI.h", "ffrKit.h", "fsdbShr.h"] {
        let path = sdk.join(header);
        fs::File::open(&path).unwrap_or_else(|e| panic!("fsdb-lib: {}: {e}", path.display()));
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let lib = ["linux64", "LINUX64"]
        .into_iter()
        .map(|name| sdk.join(name))
        .find(|path| path.join("libnffr.so").is_file() && path.join("libnsys.so").is_file())
        .expect("fsdb-lib: missing libnffr.so/libnsys.so in VERDI_HOME/share/FsdbReader/linux64");
    let mut inputs = Vec::new();
    for name in ["libnffr.so", "libnsys.so"] {
        let path = lib.join(name).canonicalize().unwrap();
        println!("cargo:rerun-if-changed={}", path.display());
        let elf = Command::new("readelf")
            .arg("-d")
            .arg(&path)
            .output()
            .expect("fsdb-lib: readelf (binutils) is required to check SDK linking");
        assert!(
            elf.status.success(),
            "fsdb-lib: cannot inspect {}",
            path.display()
        );
        assert!(
            !String::from_utf8_lossy(&elf.stdout).contains("(SONAME)"),
            "fsdb-lib: SDK library with SONAME is unsupported: {}",
            path.display()
        );
        let path = path.to_str().expect("fsdb-lib: SDK path must be UTF-8");
        assert!(
            !path.contains(['\n', '\r']),
            "fsdb-lib: newline in SDK path"
        );
        inputs.push(format!(
            "\"{}\"",
            path.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    cc::Build::new()
        .cpp(true)
        .std("c++11")
        .flag("-isystem")
        .flag(sdk.to_str().expect("fsdb-lib: SDK path must be UTF-8"))
        .file("native/fsdb.cpp")
        .file("native/fsdb_deps.S")
        .compile("ondas_fsdb_shim");
    // rlib link-args do not propagate an RPATH to consumers. These SDK libraries
    // have no SONAME, so a native linker script retains absolute DT_NEEDED paths.
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(
        out.join("libondas_fsdb_sdk.so"),
        format!("INPUT ({})\n", inputs.join(" ")),
    )
    .expect("fsdb-lib: write local SDK linker script");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=dylib=ondas_fsdb_sdk");
    println!("cargo:rustc-link-lib=z");
}
