#!/usr/bin/env bash
# WSL 冒烟：Xvfb + Helm(X11 后端) 能否启动并产生像素输出。
# 用法：bash /mnt/f/Y901/scripts/wsl-helm-check.sh
set -u
# 1) 确保 Xvfb :99 在跑（本环境没有文件系统 socket，只有抽象命名空间）
if ! pgrep -f 'Xvfb :99' > /dev/null; then
  Xvfb :99 -screen 0 1280x1024x24 -nolisten tcp > /tmp/xvfb.log 2>&1 &
  sleep 2
fi
cd /mnt/f/Helm-Y901 || exit 1
echo "== build =="
cargo build --release 2>&1 | tail -1
echo "== run (20s) =="
DISPLAY=:99 timeout 20 ./target/release/helm-y901 > /tmp/helm.out 2>&1
echo "---- exit=$? (124 = 常驻到超时，属正常) ----"
grep -E 'MIT-SHM|first frame|non-background|colors|panic' /tmp/helm.out | head -6
echo "---- tail ----"
tail -3 /tmp/helm.out
