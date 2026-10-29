use std::{
    collections::BTreeSet,
    env,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
};

fn reader_dbg_version(path: &Path) -> Option<u16> {
    let sections = Command::new("readelf")
        .args(["-SW"])
        .arg(path)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    if !sections.status.success() {
        return None;
    }
    let sections = String::from_utf8_lossy(&sections.stdout);
    let (address, offset, size) = sections.lines().find_map(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        let index = fields.iter().position(|field| *field == ".rodata")?;
        let hex = |at: usize| u64::from_str_radix(fields.get(at)?, 16).ok();
        Some((hex(index + 2)?, hex(index + 3)?, hex(index + 4)?))
    })?;
    let code = Command::new("objdump")
        .args([
            "-d",
            "--disassemble=_ZN9SimBridge12DbgReaderImp13readDbgHeaderEv",
        ])
        .arg(path)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    if !code.status.success() {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let mut versions = BTreeSet::new();
    // Read only constants referenced by the SDK's header reader. Other DBG
    // strings can describe backwards compatibility instead of its own version.
    for line in String::from_utf8_lossy(&code.stdout).lines() {
        let Some(reference) = line
            .split_once('#')
            .and_then(|(_, text)| u64::from_str_radix(text.split_whitespace().next()?, 16).ok())
        else {
            continue;
        };
        let Some(relative) = reference.checked_sub(address) else {
            continue;
        };
        if relative.checked_add(20)? > size {
            continue;
        }
        file.seek(SeekFrom::Start(offset.checked_add(relative)?))
            .ok()?;
        let mut signature = [0; 20];
        file.read_exact(&mut signature).ok()?;
        if signature.starts_with(b"Xilinx ISim DBG ")
            && signature[16..19].iter().all(u8::is_ascii_digit)
            && signature[19] == 0
        {
            versions.insert(std::str::from_utf8(&signature[16..19]).ok()?.parse().ok()?);
        }
    }
    (versions.len() == 1).then(|| versions.into_iter().next().unwrap())
}

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
    let family = ["xv", "rdi"]
        .into_iter()
        .find(|family| {
            ["wavedata", "simbridge_kernel"]
                .iter()
                .all(|name| lib.join(format!("lib{family}_{name}.so")).is_file())
        })
        .expect("wdb-lib: missing a complete xv or rdi reader/kernel pair");
    let reader = format!("lib{family}_wavedata.so");
    let kernel = format!("lib{family}_simbridge_kernel.so");
    let mut build = cc::Build::new();
    build.define(
        "ONDAS_WDB_BIT_INDEX",
        if family == "rdi" { "1" } else { "0" },
    );
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
            reader.as_str(),
            "ONDAS_WDB_READER",
            "_ZN9SimBridge10DataReader13getDataObjMgrEv",
        ),
        (
            kernel.as_str(),
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
        if name == reader {
            let version = reader_dbg_version(&path).unwrap_or(0);
            println!("cargo:rustc-env=ONDAS_WDB_DBG_VERSION={version}");
        }
        let symbols = Command::new("nm")
            .args(["-D", "--defined-only"])
            .arg(&path)
            .output()
            .expect("wdb-lib: nm (binutils) is required");
        assert!(symbols.status.success(), "wdb-lib: cannot inspect reader");
        let listing = String::from_utf8_lossy(&symbols.stdout);
        if name == reader {
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
                        line.contains("(NEEDED)") && line.contains(&format!("[{kernel}]"))
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
    println!("cargo:rustc-link-lib=dylib={family}_wavedata");
    println!("cargo:rustc-link-lib=dl");
}
