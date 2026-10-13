#[cfg(feature = "shm-lib")]
#[path = "tools/repo/shm_build.rs"]
mod shm_build;

fn main() {
    #[cfg(feature = "fsdb-lib")]
    fsdb();
    #[cfg(feature = "shm-lib")]
    shm();
}

#[cfg(feature = "shm-lib")]
fn shm() {
    use std::{env, fs, path::PathBuf, process::Command};
    println!("cargo:rerun-if-env-changed=XCELIUM_HOME");
    for source in [
        "native/shm.cpp",
        "native/shm.h",
        "native/shm_sst2.h",
        "native/shm_value.h",
        "tools/repo/shm_build.rs",
    ] {
        println!("cargo:rerun-if-changed={source}");
    }
    assert_eq!(
        env::var("TARGET").unwrap(),
        "x86_64-unknown-linux-gnu",
        "shm-lib requires x86_64-unknown-linux-gnu"
    );
    let home = PathBuf::from(env::var_os("XCELIUM_HOME").expect("shm-lib requires XCELIUM_HOME"))
        .canonicalize()
        .expect("shm-lib: inaccessible XCELIUM_HOME");
    let library = home
        .join("tools/lib/64bit/libsst2.so")
        .canonicalize()
        .expect("shm-lib: missing XCELIUM_HOME/tools/lib/64bit/libsst2.so");
    println!("cargo:rerun-if-changed={}", library.display());
    let symbols = Command::new("nm")
        .args(["-D", "--defined-only"])
        .arg(&library)
        .output()
        .expect("shm-lib: nm (binutils) is required");
    assert!(
        symbols.status.success(),
        "shm-lib: cannot inspect {}",
        library.display()
    );
    let listing = String::from_utf8_lossy(&symbols.stdout);
    let exports = shm_build::exports(&listing);
    let selection = shm_build::select(&exports).unwrap_or_else(|required| {
        panic!(
            "shm-lib: {} is missing required symbol {required}; this reader ABI is not supported",
            library.display()
        )
    });
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").file("native/shm.cpp");
    if selection.cpp_init {
        build.define("ONDAS_SST2_INIT_CPP", None);
    }
    if selection.alias_identity {
        build.define("ONDAS_SHM_USE_ALIAS_IDENTITY", None);
    }
    build.compile("ondas_shm_shim");
    println!(
        "cargo:rustc-env=ONDAS_SHM_ALIAS_PATH={}",
        if selection.alias_identity {
            "shared"
        } else {
            "separate"
        }
    );
    let path = library
        .to_str()
        .expect("shm-lib: library path must be UTF-8");
    assert!(
        !path.contains(['\n', '\r']),
        "shm-lib: newline in library path"
    );
    println!("cargo:rustc-env=ONDAS_SHM_READER={path}");
    let quoted = path.replace('\\', "\\\\").replace('"', "\\\"");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(
        out.join("libondas_shm_reader.so"),
        format!("INPUT (\"{quoted}\")\n"),
    )
    .expect("shm-lib: write reader linker script");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=dylib=ondas_shm_reader");
    println!("cargo:rustc-link-lib=z");
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
