cargo run -- test.md > /tmp/spin.log 2>&1 &
PID=$!
sleep 1
# Simulate making it dirty: we can't easily send keystrokes, but we can verify the code!
