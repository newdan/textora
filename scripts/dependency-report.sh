#!/usr/bin/env bash
set -euo pipefail

# 生成 workspace 依赖重复报告
# 用法: ./scripts/dependency-report.sh

cargo tree --workspace --duplicates
