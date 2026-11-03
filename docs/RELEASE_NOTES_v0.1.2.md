# v0.1.2 发布说明

## 修复
- `rpc_dead_connection` 在套接字资源紧张时假红：改为测试内重试 3 次（间隔 50ms），生产路径的 bind 失败仍如实上报
- README 快速开始在干净目录验证通过（首帧 9.1ms）

## 发布后跟进（不阻塞）
- P2: `cargo audit` 未安装，CI 待办
- P2: X11 真机（非 Xvfb）验证：光标 BadFont、剪贴板往返
- P3: 物理显示器上测 Present（不受 vsync 影响的真实数字）

## 状态
- tags: v0.1.0, v0.1.1, v0.1.2
- 728 默认 / 766 含 ignored，clippy 由 `scripts/gate.ps1` 验证（当前 0 警告），missing_docs 0
- 双后端：Windows D3D11 + Linux X11 CPU+SHM

## 更正（2026-11-03）

本版本声明的“clippy 0/0”是**测量伪影**。真实情况是 v0.1.3 之前存在 **92 个 clippy 警告**
（82 个 `duplicate_underscore_argument`，10 个 `to_string_in_format_args`）。
原因有两层：（a）clippy 检查**根本没进门禁脚本**，靠手打；
（b）手打的正则 `^warning: [a-z]` 匹配不到以反引号开头的消息。
已在 v0.1.4 清零，并由 `scripts/gate.ps1` 验证。
