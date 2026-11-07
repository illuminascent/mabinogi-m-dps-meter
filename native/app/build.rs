use std::{env,path::PathBuf,process::Command};
fn main(){
    println!("cargo:rerun-if-changed=assets/app.ico");println!("cargo:rerun-if-changed=assets/app.manifest");
    let target=env::var("TARGET").unwrap();if !target.contains("windows"){return;}
    let root=PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());let out=PathBuf::from(env::var("OUT_DIR").unwrap());
    let rc=out.join("app.rc");let icon=root.join("assets/app.ico").to_string_lossy().replace('\\',"/");let manifest=root.join("assets/app.manifest").to_string_lossy().replace('\\',"/");
    let version=env::var("CARGO_PKG_VERSION").unwrap();
    let parts:Vec<u16>=version.split('.').map(|p|p.parse().expect("numeric release version")).collect();
    assert_eq!(parts.len(),3,"release version must have three components");
    let (major,minor,patch)=(parts[0],parts[1],parts[2]);
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
      VALUE "FileDescription", "Mabinogi Mobile DPS Meter\0"
      VALUE "FileVersion", "{version}.0\0"
      VALUE "InternalName", "mabi-meter\0"
      VALUE "OriginalFilename", "mabi-meter.exe\0"
      VALUE "ProductName", "Mabinogi Mobile DPS Meter\0"
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
