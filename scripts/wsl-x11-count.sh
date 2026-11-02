cd /mnt/f/Y901
DISPLAY=:99 cargo test -p y901-backend-x11 > /tmp/x11.out 2>&1
echo "exit=$?"
grep -E "test result:" /tmp/x11.out | awk "{p+=\$4; f+=\$6} END {print \"total passed=\" p \" failed=\" f}"
