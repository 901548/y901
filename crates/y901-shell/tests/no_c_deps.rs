//! **零 C/C++ 依赖守卫**（Phase 0）。
//!
//! 目的：任何人往依赖树里塞一个"需要 C 编译器"的 crate，这个测试必须变红。
//!
//! 两道独立检查：
//! 1. **名字规则**：黑名单（`ring` / `openssl-sys` / `libz-sys` …）直接报错；
//!    `-sys` crate 只能出现在白名单里。
//! 2. **源码规则**：遍历每个 crate 的 `build.rs`，出现 `cc::Build` / `cmake::Config` / `bindgen::`
//!    就报错。这比"只看名字"强：有些 crate 名字人畜无害，但 build.rs 里偷偷调 cc。
//!
//! 为什么不直接读 `cargo metadata` 的 JSON：
//! 本仓库没有 JSON 数据库（也不想为一个守卫测试引进一个），
//! 而 `cargo tree` 的文本输出足够精确；build.rs 则直接按路径读取。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// **黑名单**：出现即失败。
///
/// 分类说明：
/// - `ring` / `aws-lc-*`：含 C + 汇编（这两个是 Rust TLS 生态里最容易被"以为是纯 Rust"带进来的）；
/// - `*-sys`：绑定外部 C 库（openssl / zlib / libssh2 / sqlite3 …）；
/// - `cc` / `cmake` / `bindgen`：它们本身就是 C 构建工具链的包装。
const BLACKLIST: &[&str] = &[
    "ring",
    "aws-lc-rs",
    "aws-lc-sys",
    "openssl",
    "openssl-sys",
    "native-tls-openssl",
    "libz-sys",
    "libssh2-sys",
    "libsqlite3-sys",
    "libgit2-sys",
    "libpng-sys",
    "freetype-sys",
    "harfbuzz-sys",
    "zlib-ng-sys",
    "cc",
    "cmake",
    "bindgen",
];

/// **白名单**：允许出现的 `-sys` crate。
///
/// 它们都是**纯绑定**（只有 `extern "system"` 声明 + 链接元数据），
/// 不编译任何 C。白名单变更必须改代码 → 天然形成 review 关卡。
const SYS_ALLOWLIST: &[&str] = &[
    "windows-sys",
    "windows-targets",
    "windows_x86_64_msvc",
    "windows_aarch64_msvc",
    "windows_i686_msvc",
    "windows_x86_64_gnu",
    "windows_aarch64_gnullvm",
    "windows_x86_64_gnullvm",
    "windows_i686_gnu",
    "dirs-sys",
    // wasm-bindgen JS API binding; no C source compilation.
    "js-sys",
];

fn cargo_home() -> PathBuf {
    std::env::var_os("CARGO_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".cargo")))
        .unwrap_or_else(|| PathBuf::from(".cargo"))
}

/// 从 `cargo tree` 取依赖图里的所有 crate 名及已解析版本。
fn resolve_crate_versions(manifest_dir: &Path) -> BTreeMap<String, BTreeSet<String>> {
    match try_resolve_crate_versions(manifest_dir) {
        Ok(versions) => versions,
        Err(stderr) => panic!("cargo tree 失败：{stderr}"),
    }
}

/// 同 `resolve_crate_versions`，但把失败原因交回调用方。
fn try_resolve_crate_versions(
    manifest_dir: &Path,
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    // `--target all` 会列出**所有平台**的依赖（这正是守卫要的覆盖面），但 CI 的干净
    // runner 只下载了本机 target 的 crate —— 加 `--offline` 会直接失败：
    //   failed to download `futures-core v0.3.34` / attempting to make an HTTP request…
    // 所以离线与否跟其它脚本统一：本机默认离线，`Y901_ONLINE=1` 时允许联网拉取。
    let offline = std::env::var_os("Y901_ONLINE").is_none();
    let mut args = vec!["tree", "--workspace", "--target", "all", "--prefix", "none"];
    if offline {
        args.push("--offline");
    }
    let out = Command::new(env!("CARGO"))
        .args(&args)
        .current_dir(manifest_dir)
        .output()
        .expect("需要 cargo 在 PATH 里");
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    Ok(parse_crate_versions(&String::from_utf8_lossy(&out.stdout)))
}

/// 解析 `cargo tree` 默认输出的 `crate v1.2.3`，按实际版本定位源码。
fn parse_crate_versions(text: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut versions = BTreeMap::<String, BTreeSet<String>>::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(name), Some(version)) = (fields.next(), fields.next()) else {
            continue;
        };
        versions
            .entry(name.to_string())
            .or_default()
            .insert(version.to_string());
    }
    versions
}

/// 按 cargo tree 解析出的每个精确版本，定位对应源码目录。
fn find_crate_dir(name: &str, version: &str, workspace_root: &Path) -> Option<PathBuf> {
    let version = version.strip_prefix('v').unwrap_or(version);
    // workspace 内部 crate 名由 Cargo.toml package.name 决定，路径未必与名字相同。
    if let Some(path) = find_workspace_crate(name, version, workspace_root) {
        return Some(path);
    }
    // Registry 缓存可能含有同一 crate 的多个版本，必须精确匹配每个已解析版本。
    let src = cargo_home().join("registry").join("src");
    let expected = format!("{name}-{version}");
    let roots = std::fs::read_dir(src).ok()?;
    for root in roots.flatten() {
        let candidate = root.path().join(&expected);
        if candidate.join("Cargo.toml").exists() {
            return Some(candidate);
        }
    }
    None
}

/// 按包名和版本在本仓 workspace manifests 中找本地 crate。
fn find_workspace_crate(name: &str, version: &str, workspace_root: &Path) -> Option<PathBuf> {
    let workspace_version = read_workspace_version(workspace_root)?;
    let mut manifests = Vec::new();
    walk_manifests(&workspace_root.join("crates"), &mut manifests);
    for manifest in manifests {
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let (mut in_package, mut package_name, mut package_version) = (false, None, None);
        for line in text.lines() {
            // **去掉 BOM**：编辑器加过 BOM 的 manifest 首行是 `\u{FEFF}[package]`，
            // 不剥掉会让 `starts_with('[')` 判假 → 找不到本地 crate → 误报"源码缺失"。
            let line = line
                .trim()
                .trim_start_matches(char::from_u32(0xFEFF).unwrap());
            if line.starts_with('[') {
                in_package = line == "[package]";
            } else if in_package {
                if let Some(value) = manifest_string_value(line, "name") {
                    package_name = Some(value);
                } else if line.starts_with("version.workspace") {
                    package_version = Some(workspace_version.clone());
                } else if let Some(value) = manifest_string_value(line, "version") {
                    package_version = Some(value.to_string());
                }
            }
        }
        if package_name == Some(name) && package_version.as_deref() == Some(version) {
            return manifest.parent().map(Path::to_path_buf);
        }
    }
    None
}

fn read_workspace_version(workspace_root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(workspace_root.join("Cargo.toml")).ok()?;
    let mut in_workspace_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_workspace_package = line == "[workspace.package]";
        } else if in_workspace_package {
            if let Some(value) = manifest_string_value(line, "version") {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn manifest_string_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key)?
        .trim_start()
        .strip_prefix('=')?
        .trim_start()
        .strip_prefix('"')?
        .split('"')
        .next()
}

fn walk_manifests(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.file_name().is_some_and(|n| n != "target") {
            walk_manifests(&path, out);
        } else if path.file_name().is_some_and(|n| n == "Cargo.toml") {
            out.push(path);
        }
    }
}

/// 工作区根（`crates/y901-app` 上两层）。
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("应有工作区根")
        .to_path_buf()
}

/// 扫一个 `build.rs`，命中 `cc`/`cmake`/`bindgen` 就记一条违规。
fn scan_build_rs(path: &Path, violations: &mut Vec<String>) {
    // 注意：同一个文件可能被"仓库内扫描"与"按 crate 名定位"各命中一次，
    // 这里用路径去重，免得一个违规报两条。
    if violations
        .iter()
        .any(|v| v.contains(&path.display().to_string()))
    {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for pat in ["cc::Build", "cc::build", "cmake::Config", "bindgen::"] {
        if text.contains(pat) {
            violations.push(format!(
                "【build.rs 调用 C 工具链】{} 含 `{pat}`",
                path.display()
            ));
            break;
        }
    }
}

/// 对**一个独立仓库**做完整审计，返回违规清单。
fn audit(manifest_dir: &Path) -> (usize, usize, Vec<String>) {
    let versions = resolve_crate_versions(manifest_dir);
    audit_with_versions(manifest_dir, &versions)
}

/// 审计已解析的依赖版本。
fn audit_with_versions(
    manifest_dir: &Path,
    versions: &BTreeMap<String, BTreeSet<String>>,
) -> (usize, usize, Vec<String>) {
    let mut violations: Vec<String> = Vec::new();
    let mut scanned = 0usize;

    // 检查 1：名字规则。
    for name in versions.keys() {
        if BLACKLIST.contains(&name.as_str()) {
            violations.push(format!(
                "【黑名单】{}: {name}（需要 C/C++ 工具链或外部 C 库）",
                manifest_dir.display()
            ));
        }
        if name.ends_with("-sys") && !SYS_ALLOWLIST.contains(&name.as_str()) {
            violations.push(format!(
                "【未白名单 -sys】{}: {name}（若确是纯绑定，请加入 SYS_ALLOWLIST）",
                manifest_dir.display()
            ));
        }
    }

    // 检查 2a：本仓库自己的 build.rs。
    let mut in_repo = Vec::new();
    walk_build_scripts(&manifest_dir.join("crates"), &mut in_repo);
    walk_build_scripts(&manifest_dir.join("src"), &mut in_repo);
    for path in &in_repo {
        scan_build_rs(path, &mut violations);
        scanned += 1;
    }

    // 检查 2b：逐个依赖名+精确版本扫描 build.rs；源码缺失即失败关闭。
    for (name, crate_versions) in versions {
        for version in crate_versions {
            let Some(dir) = find_crate_dir(name, version, manifest_dir) else {
                violations.push(format!(
                    "【源码缺失】{name} {version}：无法定位 Cargo 解析版本的源码"
                ));
                continue;
            };
            let build_rs = dir.join("build.rs");
            if !build_rs.exists() {
                continue;
            }
            scanned += 1;
            scan_build_rs(&build_rs, &mut violations);
        }
    }

    (versions.len(), scanned, violations)
}

/// 递归收集某目录下所有 `build.rs`。
fn walk_build_scripts(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            walk_build_scripts(&p, out);
        } else if p.file_name().is_some_and(|n| n == "build.rs") {
            out.push(p);
        }
    }
}

#[test]
fn no_c_or_cpp_toolchain_dependencies() {
    let root = workspace_root();
    let (crates, scanned, violations) = audit(&root);
    assert!(crates > 5, "依赖树解析异常，只有 {crates} 个 crate");
    eprintln!(
        "[守卫] Y901：{crates} 个 crate，扫描 build.rs {scanned} 个，违规 {}条",
        violations.len()
    );
    assert!(
        violations.is_empty(),
        "
零 C/C++ 依赖守卫失败（{}条）：
{}",
        violations.len(),
        violations.join(
            "
"
        )
    );
}

// 跨仓审计已移出框架：框架只审计自身依赖，不感知任何兄弟仓库目录。
