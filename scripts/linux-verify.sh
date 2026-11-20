#!/bin/bash
# Linux 侧验收（交付前必跑）：x11 编译/协议/真窗口 + 跨平台 crate 的字体敏感用例。
#
# 为什么需要脚本而不是几条命令：Windows 的 cargo test 不会编译 #[cfg(unix)] 代码，
# 而 cargo check --all-targets 又不链接、不跑运行时。Linux 是唯一能同时覆盖两者的探针。
#
# 用法：
#   scripts/linux-verify.sh            # 需要网络（cargo 解析整个 workspace）
#   CARGO_FLAGS=--offline scripts/linux-verify.sh   # 依赖已缓存时
#   SKIP_SLOW=1 scripts/linux-verify.sh             # 跳过 ui/widgets 全量（只跑 x11 专项）
#
# 退出码：0 = 全部通过；1 = 有失败（会逐项打印 EXIT_*）。
set -u

cd "$(dirname "$0")/.." || exit 1
if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi

CARGO_FLAGS="${CARGO_FLAGS:-}"
SLOW="${SKIP_SLOW:-0}"
FAILED=0
declare -a RESULTS=()

run() {
    local name="$1"; shift
    echo
    echo "=== $name ==="
    echo "\$ $*"
    "$@" 2>&1 | tail -n 20
    local code="${PIPESTATUS[0]}"
    echo "EXIT_$name=$code"
    RESULTS+=("$name=$code")
    if [ "$code" -ne 0 ]; then FAILED=1; fi
}

# 1) x11 后端：编译所有 target（含测试、example）
run CHECK_X11 cargo check -p y901-backend-x11 --all-targets $CARGO_FLAGS

# 2) x11 协议 golden（无 X 服务器也能跑）
run GOLDEN cargo test -p y901-backend-x11 --test golden $CARGO_FLAGS

# 3) x11 真窗口：必须 Xvfb（Xvfb 对光标形状返回 BadFont，属已知限制）
if command -v xvfb-run >/dev/null 2>&1; then
    run REAL_WINDOW xvfb-run -a -s "-screen 0 1280x800x24" \
        cargo test -p y901-backend-x11 --test real_window $CARGO_FLAGS
else
    echo "[linux-verify] WARN: 没有 xvfb-run，跳过 real_window（结果不能算通过）"
    RESULTS+=("REAL_WINDOW=SKIP")
    FAILED=1
fi

# 4) Soft 描边：曾长期不在 Linux 验收范围内（STATUS.md 已记录该缺口）
run STROKE cargo test -p y901-render --test stroke_round_aa $CARGO_FLAGS

# 5) SGR 反色：按"格内背景众数"断言，不依赖具体字体的字形覆盖范围
run SGR_REVERSE cargo test -p y901-extras --test sgr_reverse $CARGO_FLAGS

# 6) 跨平台 crate 全量（可选；覆盖本次改动涉及的 ui/widgets 逻辑）
if [ "$SLOW" != "1" ]; then
    run UI_WIDGETS cargo test -p y901-ui -p y901-widgets $CARGO_FLAGS
fi

# 7) 全量 workspace（默认跑；SKIP_FULL=1 可跳过）
#
# 为什么把结果写回仓库：Linux 的计数是**独立数据源**（`docs/evidence/linux-numbers.txt`），
# Windows 的 `sync-doc-numbers.ps1 -Check` 用它校验文档里的 Linux 行。
# 不写回去，Linux 的数字就只能靠人肉比对日志。
if [ "${SKIP_FULL:-0}" != "1" ]; then
    FULL_LOG=/tmp/y901-linux-full.log
    echo
    echo "=== FULL_WORKSPACE ==="
    echo "\$ xvfb-run -a cargo test --workspace --no-fail-fast $CARGO_FLAGS"
    xvfb-run -a -s "-screen 0 1280x800x24" \
        cargo test --workspace --no-fail-fast $CARGO_FLAGS >"$FULL_LOG" 2>&1
    FULL_CODE=$?
    tail -n 20 "$FULL_LOG"
    echo "EXIT_FULL_WORKSPACE=$FULL_CODE"
    RESULTS+=("FULL_WORKSPACE=$FULL_CODE")
    if [ "$FULL_CODE" -ne 0 ]; then
        FAILED=1
        grep -E '^test .* FAILED|test result: FAILED' "$FULL_LOG" | head -n 20 || true
    else
        PASSED=$(grep 'test result:' "$FULL_LOG" | sed -n 's/.* \([0-9]\+\) passed;.*/\1/p' | awk '{s+=$1} END{print s+0}')
        IGNORED=$(grep 'test result:' "$FULL_LOG" | sed -n 's/.* \([0-9]\+\) ignored;.*/\1/p' | awk '{s+=$1} END{print s+0}')
        TARGETS=$(grep -c 'test result:' "$FULL_LOG")
        NUMBERS=docs/evidence/linux-numbers.txt
        mkdir -p "$(dirname "$NUMBERS")"
        {
            echo "# Linux 侧计数（由 scripts/linux-verify.sh 在 Linux 上生成，Windows 的 sync 只读不写）"
            echo "#"
            echo "# 为什么单独一个文件：numbers.txt 是 Windows 门禁口径，而 Linux 的计数只能在"
            echo "# Linux 上跑出来。两者分开存，sync-doc-numbers.ps1 -Check 才能分别校验"
            echo "# \"Windows 行\"与\"Linux 行\"，而不是把 Linux 行整行豁免（那会让它的漂移不可见）。"
            echo "#"
            echo "# 生成：在 Linux 上 bash scripts/linux-verify.sh（默认会跑全量 workspace 测试）"
            echo "# 会把结果写回仓库里的这个文件，随后把文件带回 Windows 提交。"
            echo "passed=$PASSED"
            echo "ignored=$IGNORED"
            echo "targets=$TARGETS"
        } >"$NUMBERS"
        echo "[linux-verify] wrote $NUMBERS (passed=$PASSED ignored=$IGNORED targets=$TARGETS)"
        echo "[linux-verify] REMINDER: copy this file back to the Windows checkout and commit it."
    fi
fi

echo
echo "=== 汇总 ==="
for r in "${RESULTS[@]}"; do echo "$r"; done
if [ "$FAILED" -ne 0 ]; then
    echo "LINUX VERIFY: FAILED"
    exit 1
fi
echo "LINUX VERIFY: OK"
