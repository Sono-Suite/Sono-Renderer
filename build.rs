use std::process::Command;

fn main() {
    generate_locale_resources();
    for name in [
        "SONO_RENDERER_GIT_COMMIT",
        "SONO_RENDERER_BUILD_CHANNEL",
        "SONO_RENDERER_BUILD_DIRTY",
        "SONO_RENDERER_REQUIRE_WIN_RESOURCE",
        "RC",
        "GITHUB_SHA",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    let commit = std::env::var("SONO_RENDERER_GIT_COMMIT")
        .or_else(|_| std::env::var("GITHUB_SHA"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(git_commit)
        .unwrap_or_else(|| "unknown".into());
    let commit = commit.chars().take(12).collect::<String>();
    let dirty = std::env::var("SONO_RENDERER_BUILD_DIRTY")
        .ok()
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or_else(git_worktree_is_dirty);
    let channel =
        std::env::var("SONO_RENDERER_BUILD_CHANNEL").unwrap_or_else(|_| "development".into());
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown-target".into());
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown-profile".into());
    let build_id = format!(
        "{commit}{} · {channel} · {target}",
        if dirty { "+dirty" } else { "" }
    );

    for (name, value) in [
        ("SONO_RENDERER_GIT_COMMIT", commit),
        (
            "SONO_RENDERER_BUILD_DIRTY",
            if dirty { "true" } else { "false" }.into(),
        ),
        ("SONO_RENDERER_BUILD_CHANNEL", channel),
        ("SONO_RENDERER_BUILD_TARGET", target.clone()),
        ("SONO_RENDERER_BUILD_PROFILE", profile.clone()),
        ("SONO_RENDERER_BUILD_ID", build_id.clone()),
    ] {
        println!("cargo:rustc-env={name}={value}");
    }

    if target.ends_with("windows-msvc") {
        embed_windows_version_info(
            &std::env::var("CARGO_PKG_VERSION").expect("Cargo supplies package version"),
            &profile,
            &build_id,
            dirty,
        );
    }
}

fn generate_locale_resources() {
    let manifest = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo supplies manifest directory"),
    );
    let root = manifest.join("locales");
    println!("cargo:rerun-if-changed={}", root.display());
    let registry = root.join("registry.json");
    println!("cargo:rerun-if-changed={}", registry.display());
    let mut catalogs = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        panic!("locales/ is required to build Sono-Renderer");
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let directory = entry.file_name().to_string_lossy().to_string();
        if directory == "test" {
            let Ok(test_entries) = std::fs::read_dir(&path) else {
                continue;
            };
            for test_entry in test_entries.flatten() {
                let test_path = test_entry.path().join("messages.json");
                if test_path.is_file() {
                    let id = match test_entry.file_name().to_string_lossy().as_ref() {
                        "pseudo" => "qps-ploc".to_owned(),
                        "rtl" => "qps-rtl".to_owned(),
                        other => format!("test-{other}"),
                    };
                    println!("cargo:rerun-if-changed={}", test_path.display());
                    catalogs.push((id, test_path));
                }
            }
        } else {
            let catalog_path = path.join("messages.json");
            if catalog_path.is_file() {
                println!("cargo:rerun-if-changed={}", catalog_path.display());
                catalogs.push((directory, catalog_path));
            }
        }
    }
    catalogs.sort_by(|left, right| left.0.cmp(&right.0));
    for (id, path) in &catalogs {
        let metadata = std::fs::metadata(path).expect("locale catalog metadata is readable");
        if metadata.len() > 2 * 1024 * 1024 {
            panic!("locale catalog {id} exceeds the 2 MiB build-time limit");
        }
    }
    let out_dir = std::path::PathBuf::from(
        std::env::var_os("OUT_DIR").expect("Cargo supplies build output directory"),
    );
    let generated = out_dir.join("sono_renderer_locales.rs");
    let mut source = String::from("pub const LOCALE_REGISTRY_JSON: &str = include_str!(\"");
    source.push_str(&registry.display().to_string().replace('\\', "\\\\"));
    source.push_str("\");\npub const BUILTIN_LOCALE_CATALOGS: &[(&str, &str)] = &[\n");
    for (id, path) in catalogs {
        source.push_str(&format!("    ({id:?}, include_str!({:?})),\n", path));
    }
    source.push_str("];\n");
    std::fs::write(generated, source).expect("write generated locale resource index");
}

fn embed_windows_version_info(version: &str, profile: &str, build_id: &str, dirty: bool) {
    let required = std::env::var("SONO_RENDERER_REQUIRE_WIN_RESOURCE")
        .is_ok_and(|value| value.eq_ignore_ascii_case("true"));
    let compiler = std::env::var_os("RC")
        .map(std::path::PathBuf::from)
        .or_else(|| find_executable("rc.exe"));
    let Some(compiler) = compiler else {
        if required {
            panic!("Windows release requires rc.exe to embed executable version metadata");
        }
        println!("cargo:warning=rc.exe unavailable; Windows executable version metadata was not embedded");
        return;
    };

    let numeric_version = version.split(['-', '+']).next().unwrap_or(version);
    let parts = numeric_version
        .split('.')
        .map(|part| part.parse::<u16>())
        .collect::<Result<Vec<_>, _>>()
        .expect("Cargo package version must have numeric components");
    let major = parts.first().copied().unwrap_or(0);
    let minor = parts.get(1).copied().unwrap_or(0);
    let patch = parts.get(2).copied().unwrap_or(0);
    let flags = if profile == "debug" || dirty {
        "0x1L"
    } else {
        "0x0L"
    };
    let source = format!(
        r#"1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS {flags}
FILEOS 0x00040004L
FILETYPE 0x00000001L
FILESUBTYPE 0x00000000L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Nexint\0"
      VALUE "FileDescription", "Sono-Renderer\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "InternalName", "Sono-Renderer\0"
      VALUE "OriginalFilename", "Sono-Renderer.exe\0"
      VALUE "ProductName", "Sono-Renderer\0"
      VALUE "ProductVersion", "{version}\0"
      VALUE "Comments", "{build_id}\0"
      VALUE "LegalCopyright", "Copyright (C) Nexint\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#,
        version = rc_escape(version),
        build_id = rc_escape(build_id),
    );
    let out_dir = std::path::PathBuf::from(
        std::env::var_os("OUT_DIR").expect("Cargo supplies build output directory"),
    );
    let resource_file = out_dir.join("sono_renderer_version.rc");
    let compiled_file = out_dir.join("sono_renderer_version.res");
    if let Err(error) = std::fs::write(&resource_file, source) {
        if required {
            panic!("could not write Windows version resource: {error}");
        }
        println!("cargo:warning=could not write Windows executable version resource: {error}");
        return;
    }
    let result = Command::new(&compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&compiled_file)
        .arg(&resource_file)
        .output();
    match result {
        Ok(output) if output.status.success() => {
            println!(
                "cargo:rustc-link-arg-bin=renderer={}",
                compiled_file.display()
            );
        }
        Ok(output) => {
            let detail = String::from_utf8_lossy(&output.stderr);
            if required {
                panic!("rc.exe failed to compile version metadata: {detail}");
            }
            println!("cargo:warning=rc.exe failed to compile Windows executable version metadata: {detail}");
        }
        Err(error) => {
            if required {
                panic!("could not run rc.exe for Windows version metadata: {error}");
            }
            println!("cargo:warning=could not run rc.exe for Windows executable version metadata: {error}");
        }
    }
}

fn rc_escape(value: &str) -> String {
    value
        .chars()
        .map(|character| if character.is_ascii() { character } else { '?' })
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\r', '\n'], " ")
}

fn find_executable(name: &str) -> Option<std::path::PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
}

fn git_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn git_worktree_is_dirty() -> bool {
    Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=normal"])
        .output()
        .map(|output| !output.status.success() || !output.stdout.is_empty())
        .unwrap_or(true)
}
