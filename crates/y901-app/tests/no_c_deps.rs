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

use std::collections::BTreeSet;
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
];

fn cargo_home() -> PathBuf {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".cargo")))
        .unwrap_or_else(|| PathBuf::from(".cargo"))
}

/// 从 `cargo tree` 取依赖图里的所有 crate 名（去重、去版本号）。
fn resolve_crate_names(manifest_dir: &Path) -> BTreeSet<String> {
    match try_resolve_crate_names(manifest_dir) {
        Ok(names) => names,
        Err(stderr) => panic!("cargo tree 失败：{stderr}"),
    }
}

/// 同 [`resolve_crate_names`]，但把失败原因交回调用方（兄弟仓守卫要按错误类型分流）。
fn try_resolve_crate_names(manifest_dir: &Path) -> Result<BTreeSet<String>, String> {
    let out = Command::new(env!("CARGO"))
        .args(["tree", "--prefix", "none", "--offline"])
        .current_dir(manifest_dir)
        .output()
        .expect("需要 cargo 在 PATH 里");
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    Ok(parse_crate_names(&String::from_utf8_lossy(&out.stdout)))
}

/// 解析 `cargo tree --prefix none` 的文本输出，只取每行第一个词（crate 名）。
fn parse_crate_names(text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for line in text.lines() {
        // 格式：`crate v1.2.3 (可选路径)`；只取第一个词
        let name = line.split_whitespace().next().unwrap_or("");
        if name.is_empty() || name.starts_with('(') {
            continue;
        }
        names.insert(name.to_string());
    }
    names
}

/// 本仓库里**已被物理删除**的本地 crate。
///
/// 兄弟仓若还声明了它们，`cargo tree` 会以"读不到 Cargo.toml"失败——
/// 那是**跨仓迁移未完成**，不是"兄弟仓引入了 C 依赖"。
const DELETED_LOCAL_CRATES: &[&str] = &["y901-agent", "y901-mcp"];

/// 判断 `cargo tree` 的 stderr 是否属于"依赖的本地 crate 已被删除"这一类**特定**错误。
///
/// 必须同时满足：
/// 1. 错误文本含 `failed to read` 或 `failed to load source for dependency`；
/// 2. 报错路径落在本仓 `crates/` 下；
/// 3. 命中的 crate 名在 [`DELETED_LOCAL_CRATES`] 里。
///
/// 任一条件不满足都返回 `None`（调用方按普通失败处理），避免 catch-all 吞错。
fn deleted_local_dependency(stderr: &str) -> Option<&'static str> {
    let is_missing_source = stderr.contains("failed to read")
        || stderr.contains("failed to load source for dependency");
    if !is_missing_source {
        return None;
    }
    let points_into_workspace =
        stderr.contains("Y901\\crates\\") || stderr.contains("Y901/crates/");
    if !points_into_workspace {
        return None;
    }
    DELETED_LOCAL_CRATES.iter().copied().find(|name| {
        stderr.contains(&format!("crates\\{name}"))
            || stderr.contains(&format!("crates/{name}"))
            || stderr.contains(&format!("`{name}`"))
    })
}

/// 在 registry 与本地 workspace 里找到某个 crate 的源码目录（取最新版本）。
fn find_crate_dir(name: &str, workspace_root: &Path) -> Option<PathBuf> {
    // 1) workspace 内部 crate
    let local = workspace_root.join("crates").join(name);
    if local.join("Cargo.toml").exists() {
        return Some(local);
    }
    // 2) registry 源码（src/*/name-version）
    let src = cargo_home().join("registry").join("src");
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    if let Ok(dirs) = std::fs::read_dir(&src) {
        for d in dirs.flatten() {
            if let Ok(entries) = std::fs::read_dir(d.path()) {
                for e in entries.flatten() {
                    let fname = e.file_name().to_string_lossy().to_string();
                    let Some(rest) = fname.strip_prefix(&format!("{name}-")) else {
                        continue;
                    };
                    // 只接受纯数字版本（避开 `foo-bar` 这种同前缀）
                    if !rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        continue;
                    }
                    let ver: Vec<u64> = rest
                        .split(['.', '-', '+'])
                        .filter_map(|p| p.parse::<u64>().ok())
                        .collect();
                    if best.as_ref().is_none_or(|(b, _)| &ver > b) {
                        best = Some((ver, e.path()));
                    }
                }
            }
        }
    }
    best.map(|(_, p)| p)
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
///
/// 为什么抽成函数：Helm-Y901 是**另一个独立工程**，
/// 它的依赖树里塞进 `ring` 同样是违规，而守卫应当同时测两个仓。
fn audit(manifest_dir: &Path) -> (usize, usize, Vec<String>) {
    let names = resolve_crate_names(manifest_dir);
    audit_with_names(manifest_dir, &names)
}

/// [`audit`] 的主体：依赖树已由调用方解析好（兄弟仓守卫需要先按错误类型分流）。
fn audit_with_names(manifest_dir: &Path, names: &BTreeSet<String>) -> (usize, usize, Vec<String>) {
    let mut violations: Vec<String> = Vec::new();
    let mut scanned = 0usize;

    // 检查 1：名字规则
    for name in names {
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

    // 检查 2a：本仓库自己的 build.rs
    let mut in_repo = Vec::new();
    walk_build_scripts(&manifest_dir.join("crates"), &mut in_repo);
    walk_build_scripts(&manifest_dir.join("src"), &mut in_repo);
    for path in &in_repo {
        scan_build_rs(path, &mut violations);
        scanned += 1;
    }

    // 检查 2b：依赖树里每个 crate 的 build.rs
    let ws_root = manifest_dir.to_path_buf();
    for name in names {
        let Some(dir) = find_crate_dir(name, &ws_root) else {
            continue;
        };
        let build_rs = dir.join("build.rs");
        if !build_rs.exists() {
            continue;
        }
        scanned += 1;
        scan_build_rs(&build_rs, &mut violations);
    }

    (names.len(), scanned, violations)
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

/// 对兄弟仓库 `Helm-Y901` 做同样的审计（不存在时跳过，便于单独检出 Y901）。
// TODO(去 AI 阶段 3)：F:\Helm-Y901 迁移完成后删除此跳过分支
#[test]
fn sibling_repo_helm_has_no_c_dependencies() {
    let helm = workspace_root()
        .parent()
        .map(|p| p.join("Helm-Y901"))
        .unwrap_or_default();
    if !helm.join("Cargo.toml").exists() {
        eprintln!("[守卫] Helm-Y901 不在相邻目录，跳过");
        return;
    }
    // 依赖树解析失败要分两类：
    //   * 兄弟仓依赖了本仓已删除的本地 crate → 跨仓迁移未完成，显式跳过并留 TODO；
    //   * 其他任何失败（锁文件坏了 / 网络 / 真正的解析错误）→ 仍然红。
    let names = match try_resolve_crate_names(&helm) {
        Ok(names) => names,
        Err(stderr) => {
            if let Some(deleted) = deleted_local_dependency(&stderr) {
                eprintln!(
                    "[守卫] Helm-Y901 依赖树无法解析：依赖的本地 crate 已被删除（{deleted}）"
                );
                eprintln!(
                    "[守卫] 跳过 Helm 零 C 依赖审计。TODO：迁移 F:\\Helm-Y901 去除 agent/mcp 依赖后恢复本守卫。"
                );
                return;
            }
            panic!("cargo tree 失败：{stderr}");
        }
    };
    let (crates, scanned, violations) = audit_with_names(&helm, &names);
    eprintln!(
        "[守卫] Helm-Y901：{crates} 个 crate，扫描 build.rs {scanned} 个，违规 {}条",
        violations.len()
    );
    assert!(
        violations.is_empty(),
        "
Helm-Y901 守卫失败（{}条）：
{}",
        violations.len(),
        violations.join(
            "
"
        )
    );
}
