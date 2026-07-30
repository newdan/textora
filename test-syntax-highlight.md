# Syntax Highlighting Test

## Rust

```rust
fn main() {
    let name = "edit+";
    let count: u32 = 42;
    let is_active = true;

    // This is a comment
    if is_active {
        println!("Hello, {}! Count: {}", name, count);
    }

    for i in 0..10 {
        match i {
            0 => println!("zero"),
            1..=5 => println!("small"),
            _ => println!("big"),
        }
    }
}
```

## Python

```python
import os
from pathlib import Path

class FileWatcher:
    """Watch files for changes."""

    def __init__(self, root: str):
        self.root = Path(root)
        self._cache: dict[str, float] = {}

    def scan(self) -> list[str]:
        changed = []
        for f in self.root.glob("**/*.py"):
            mtime = f.stat().st_mtime
            if str(f) not in self._cache or self._cache[str(f)] < mtime:
                changed.append(str(f))
                self._cache[str(f)] = mtime
        return changed

if __name__ == "__main__":
    watcher = FileWatcher(".")
    for path in watcher.scan():
        print(f"Changed: {path}")
```

## JavaScript

```javascript
const express = require('express');
const app = express();

app.get('/api/users', async (req, res) => {
  const { page = 1, limit = 20 } = req.query;
  try {
    const users = await db.collection('users')
      .find({})
      .skip((page - 1) * limit)
      .limit(Number(limit))
      .toArray();
    res.json({ data: users, total: users.length });
  } catch (err) {
    console.error('Query failed:', err);
    res.status(500).json({ error: 'Internal server error' });
  }
});

app.listen(3000, () => console.log('Server running on :3000'));
```

## JSON

```json
{
  "name": "edit-plus",
  "version": "0.1.0",
  "features": {
    "syntax_highlighting": true,
    "markdown_preview": true,
    "cjk_support": true
  },
  "languages": ["rust", "python", "javascript", "json", "yaml", "toml"]
}
```

## Shell

```bash
#!/bin/bash
set -euo pipefail

REPO_DIR="${1:-.}"
BRANCH="main"

cd "$REPO_DIR"

if ! git diff --quiet; then
    echo "Uncommitted changes found"
    git status --short
    exit 1
fi

echo "Pulling latest from $BRANCH..."
git pull origin "$BRANCH"

# Install dependencies
if [ -f "Cargo.toml" ]; then
    cargo fetch
elif [ -f "package.json" ]; then
    npm install
fi
```

## YAML

```yaml
name: CI
on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --all
      - run: cargo clippy -- -D warnings
```

## TOML

```toml
[package]
name = "my-app"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1", features = ["full"] }

[profile.release]
opt-level = 3
lto = true
```

## Plain code block (no language tag — no highlighting expected)

```
This block has no language tag.
It should render as plain monospace text
without any syntax coloring.
```
