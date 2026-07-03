use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let root = find_ten_vad_root().unwrap_or_else(|e| panic!("{e}"));
    let header = root.join("include/ten_vad.h");
    if !header.exists() {
        panic!(
            "未找到 TEN-VAD 头文件: {}。请设置 TEN_VAD_LIB_DIR 或放置 lib/ten-vad",
            header.display()
        );
    }

    println!("cargo:rerun-if-env-changed=TEN_VAD_LIB_DIR");
    println!("cargo:rerun-if-changed={}", header.display());

    link_native(&root);
    copy_runtime_libs(native_lib_dir(&root));
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(".."))
}

fn has_native_lib(root: &Path) -> bool {
    if cfg!(target_os = "windows") {
        native_lib_dir(root).join("ten_vad.lib").exists()
    } else if cfg!(target_os = "linux") {
        native_lib_dir(root).join("libten_vad.so").exists()
    } else if cfg!(target_os = "macos") {
        root.join("lib/macOS/ten_vad.framework").exists()
    } else {
        false
    }
}

fn find_ten_vad_root() -> Result<PathBuf, String> {
    if let Ok(dir) = env::var("TEN_VAD_LIB_DIR") {
        let path = PathBuf::from(dir);
        if path.join("include/ten_vad.h").exists() && has_native_lib(&path) {
            return Ok(path);
        }
        return Err(format!(
            "TEN_VAD_LIB_DIR={} 无效，需包含 include/ten_vad.h 与当前平台的预编译库",
            path.display()
        ));
    }

    let workspace = workspace_root();
    let local = workspace.join("lib/ten-vad");
    if local.join("include/ten_vad.h").exists() && has_native_lib(&local) {
        return Ok(local);
    }

    let candidates = [workspace
        .join("..")
        .join("xiaozhi-esp32-server-golang")
        .join("lib/ten-vad")];
    for candidate in candidates {
        if candidate.join("include/ten_vad.h").exists() && has_native_lib(&candidate) {
            return Ok(candidate.canonicalize().unwrap_or(candidate));
        }
    }

    Err(
        "未找到 TEN-VAD 预编译库。请设置 TEN_VAD_LIB_DIR 指向 lib/ten-vad 根目录，\
         或从 Go 版/ten-vad 仓库复制 lib/Windows|Linux|macOS 到 workspace/lib/ten-vad"
            .into(),
    )
}

fn native_lib_dir(root: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        root.join("lib/Windows/x64")
    } else if cfg!(target_os = "linux") {
        root.join("lib/Linux/x64")
    } else if cfg!(target_os = "macos") {
        root.join("lib/macOS")
    } else {
        root.join("lib")
    }
}

fn link_native(root: &Path) {
    if cfg!(target_os = "windows") {
        let lib_dir = native_lib_dir(root);
        if !lib_dir.join("ten_vad.lib").exists() {
            panic!(
                "未找到 Windows TEN-VAD 库: {}（需要 ten_vad.lib / ten_vad.dll）",
                lib_dir.display()
            );
        }
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!("cargo:rustc-link-lib=dylib=ten_vad");
        return;
    }

    if cfg!(target_os = "linux") {
        let lib_dir = native_lib_dir(root);
        if !lib_dir.join("libten_vad.so").exists() {
            panic!(
                "未找到 Linux TEN-VAD 库: {}（需要 libten_vad.so）",
                lib_dir.display()
            );
        }
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!("cargo:rustc-link-lib=dylib=ten_vad");
        println!("cargo:rustc-link-lib=dylib=stdc++");
        println!("cargo:rustc-link-lib=dylib=c++abi");
        return;
    }

    if cfg!(target_os = "macos") {
        let framework_dir = root.join("lib/macOS");
        if !framework_dir.join("ten_vad.framework").exists() {
            panic!(
                "未找到 macOS TEN-VAD framework: {}",
                framework_dir.display()
            );
        }
        println!(
            "cargo:rustc-link-search=framework={}",
            framework_dir.display()
        );
        println!("cargo:rustc-link-lib=framework=ten_vad");
    }
}

fn target_output_dir() -> PathBuf {
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    if let Ok(dir) = env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(dir).join(profile);
    }
    workspace_root().join("target").join(profile)
}

fn copy_runtime_libs(lib_dir: PathBuf) {
    let dest_dir = target_output_dir();
    let _ = std::fs::create_dir_all(&dest_dir);

    if cfg!(target_os = "windows") {
        let dll = lib_dir.join("ten_vad.dll");
        if dll.exists() {
            let dest = dest_dir.join("ten_vad.dll");
            if std::fs::copy(&dll, &dest).is_ok() {
                println!("cargo:warning=已复制 ten_vad.dll 到 {}", dest.display());
            }
        }
        return;
    }

    if cfg!(target_os = "linux") {
        let so = lib_dir.join("libten_vad.so");
        if so.exists() {
            let dest = dest_dir.join("libten_vad.so");
            let _ = std::fs::copy(&so, &dest);
        }
    }
}
