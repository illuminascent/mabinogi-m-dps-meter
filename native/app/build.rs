use std::{env,path::PathBuf,process::Command};
fn main(){
    // Content identity works in offline Docker builds without a mounted .git.
    // This is a reproducibility fingerprint, not a cryptographic authenticity check.
    let source_root=PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).parent().unwrap().to_path_buf();
    let mut files=Vec::new();
    fn gather(root:&std::path::Path,files:&mut Vec<PathBuf>){
        for entry in std::fs::read_dir(root).expect("source fingerprint directory") {
            let p=entry.expect("source fingerprint entry").path();
            if p.is_dir(){gather(&p,files);}else{files.push(p);}
        }
    }
    for part in ["core/src","core/data","app/src","app/assets","capture/src"] {gather(&source_root.join(part),&mut files);}
    for part in ["Cargo.toml","Cargo.lock","core/Cargo.toml","app/Cargo.toml","app/build.rs","capture/Cargo.toml"] {files.push(source_root.join(part));}
    files.push(source_root.parent().unwrap().join("assets/data/combat-names.json"));
    files.sort();let mut fingerprint=0xcbf29ce484222325u64;
    for path in files {
        println!("cargo:rerun-if-changed={}",path.display());
        let relative=path.strip_prefix(source_root.parent().unwrap()).unwrap().to_string_lossy().replace('\\',"/");
        for b in relative.as_bytes().iter().copied().chain([0]).chain(std::fs::read(&path).expect("source fingerprint input")) {
            fingerprint=(fingerprint^u64::from(b)).wrapping_mul(0x100000001b3);
        }
    }
    println!("cargo:rustc-env=MABI_SOURCE_FINGERPRINT=fnv1a64:{fingerprint:016x}");
    println!("cargo:rerun-if-changed=assets/app.ico");println!("cargo:rerun-if-changed=assets/app.manifest");
    let target=env::var("TARGET").unwrap();if !target.contains("windows"){return;}
    let root=PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());let out=PathBuf::from(env::var("OUT_DIR").unwrap());
    let rc=out.join("app.rc");let icon=root.join("assets/app.ico").to_string_lossy().replace('\\',"/");let manifest=root.join("assets/app.manifest").to_string_lossy().replace('\\',"/");
    let version=env::var("CARGO_PKG_VERSION").unwrap();
    let parts:Vec<u16>=version.split('.').map(|p|p.parse().expect("numeric release version")).collect();
    assert_eq!(parts.len(),3,"release version must have three components");
    let (major,minor,patch)=(parts[0],parts[1],parts[2]);
    let research=env::var_os("CARGO_FEATURE_RESEARCH_UI").is_some();
    let name=if research {"mabi-research"} else {"mabi-meter"};
    let description=if research {"Mabinogi Mobile Research Capture"} else {"Mabinogi Mobile DPS Meter"};
    std::fs::write(&rc,format!(r#"1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "{description}\0"
      VALUE "FileVersion", "{version}.0\0"
      VALUE "InternalName", "{name}\0"
      VALUE "OriginalFilename", "{name}.exe\0"
      VALUE "ProductName", "{description}\0"
      VALUE "ProductVersion", "{version}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#)).expect("write resource script");
    let object=out.join(if target.ends_with("gnu"){"app-resource.o"}else{"app-resource.res"});
    let status=if target.ends_with("gnu"){
        Command::new(env::var("WINDRES").unwrap_or_else(|_|"x86_64-w64-mingw32-windres".into())).args(["-i"]).arg(&rc).args(["-O","coff","-o"]).arg(&object).status()
    }else{Command::new("rc.exe").arg(format!("/fo{}",object.display())).arg(&rc).status()}.expect("Windows resource compiler is required to embed the application icon");
    assert!(status.success(),"compile application icon/manifest");println!("cargo:rustc-link-arg={}",object.display());
}
