use std::{env,path::PathBuf,process::Command};
fn main(){
    println!("cargo:rerun-if-changed=assets/app.ico");println!("cargo:rerun-if-changed=assets/app.manifest");
    let target=env::var("TARGET").unwrap();if !target.contains("windows"){return;}
    let root=PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());let out=PathBuf::from(env::var("OUT_DIR").unwrap());
    let rc=out.join("app.rc");let icon=root.join("assets/app.ico").to_string_lossy().replace('\\',"/");let manifest=root.join("assets/app.manifest").to_string_lossy().replace('\\',"/");
    std::fs::write(&rc,format!("1 ICON \"{icon}\"\n1 24 \"{manifest}\"\n")).expect("write resource script");
    let object=out.join(if target.ends_with("gnu"){"app-resource.o"}else{"app-resource.res"});
    let status=if target.ends_with("gnu"){
        Command::new(env::var("WINDRES").unwrap_or_else(|_|"x86_64-w64-mingw32-windres".into())).args(["-i"]).arg(&rc).args(["-O","coff","-o"]).arg(&object).status()
    }else{Command::new("rc.exe").arg(format!("/fo{}",object.display())).arg(&rc).status()}.expect("Windows resource compiler is required to embed the application icon");
    assert!(status.success(),"compile application icon/manifest");println!("cargo:rustc-link-arg={}",object.display());
}
