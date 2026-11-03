# v0.1.0 发布说明（补录）

原始发布时未留 release notes 文件，此处补录当时声明的状态与后续更正。

## 更正（2026-11-03）

本版本声明的“clippy 0/0”是**测量伪影**。真实情况是 v0.1.3 之前存在 **92 个 clippy 警告**
（82 个 `duplicate_underscore_argument`，10 个 `to_string_in_format_args`）。
原因有两层：（a）clippy 检查**根本没进门禁脚本**，靠手打；
（b）手打的正则 `^warning: [a-z]` 匹配不到以反引号开头的消息。
已在 v0.1.4 清零，并由 `scripts/gate.ps1` 验证。
