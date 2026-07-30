# Concurrent Initialization Soundness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除安全 public API 内部无同步 `static mut` 的数据竞争，使 SIMD 分派和 ICU 懒初始化可被多线程安全调用。

**Architecture:** 无状态 SIMD 后端用 `OnceLock<fn>` 一次选择；不可确认跨线程共享安全性的 ICU handle 使用 thread-local `OnceCell`，编码表使用进程级 `OnceLock`。先以并发首次调用测试复现 API 契约，再逐模块替换，最后做 Miri 可覆盖路径检查。

**Tech Stack:** Rust `std::sync::OnceLock`、`std::cell::OnceCell`、线程压力测试、Miri。

---

### Task 1: 固化 SIMD 并发首次调用行为

**Files:**
- Modify: `crates/core/src/simd/mod.rs`
- Modify: `crates/stdext/src/simd/mod.rs`

- [ ] **Step 1: 增加 core SIMD 并发压力测试**

在测试模块加入：

```rust
#[test]
fn dispatch_initialization_is_thread_safe() {
    let input = std::sync::Arc::new(b"alpha\nbeta\r\ngamma".to_vec());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
    let handles: Vec<_> = (0..16)
        .map(|_| {
            let input = input.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..10_000 {
                    assert_eq!(super::memchr2(b'\n', b'\r', &input, 0), 5);
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
}
```

在 stdext 测试模块加入 16 线程、每线程 10,000 次调用 public memset wrapper 的等价测试，每次验证填充值完整。

- [ ] **Step 2: 运行压力测试作为变更前证据**

```bash
cargo test -p edit-plus-core --lib dispatch_initialization_is_thread_safe -- --exact
cargo test -p stdext --lib dispatch_initialization_is_thread_safe -- --exact
```

Expected: 普通运行可能 PASS；这两个测试的作用是锁定安全 API 的并发契约，不能以“当前未复现 UB”为由删除。

- [ ] **Step 3: 提交测试**

```bash
git add crates/core/src/simd/mod.rs crates/stdext/src/simd/mod.rs
git commit -m "test(simd): stress concurrent dispatch initialization"
```

### Task 2: 用 OnceLock 替换 core SIMD 自修改分派

**Files:**
- Modify: `crates/core/src/simd/memchr2.rs:54-85`
- Modify: `crates/core/src/simd/lines_fwd.rs:66-95`
- Modify: `crates/core/src/simd/lines_bwd.rs:66-95`

- [ ] **Step 1: 对每种签名定义函数类型与 OnceLock**

`memchr2.rs` 使用：

```rust
type Memchr2Fn = unsafe fn(u8, u8, *const u8, *const u8) -> *const u8;

#[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "loongarch64"))]
static MEMCHR2_DISPATCH: std::sync::OnceLock<Memchr2Fn> = std::sync::OnceLock::new();

fn selected_memchr2() -> Memchr2Fn {
    *MEMCHR2_DISPATCH.get_or_init(|| {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        if is_x86_feature_detected!("avx2") { return memchr2_avx2; }
        #[cfg(target_arch = "loongarch64")]
        if std::arch::is_loongarch_feature_detected!("lasx") { return memchr2_lasx; }
        #[cfg(target_arch = "loongarch64")]
        if std::arch::is_loongarch_feature_detected!("lsx") { return memchr2_lsx; }
        memchr2_fallback
    })
}
```

`lines_fwd.rs`/`lines_bwd.rs` 使用完整签名：

```rust
type LinesFn = unsafe fn(
    *const u8,
    *const u8,
    CoordType,
    CoordType,
) -> (*const u8, CoordType);
```

两文件各自定义 `OnceLock<LinesFn>` 与 `selected_lines_fwd`/`selected_lines_bwd`，选择逻辑和当前 x86_64/loongarch64 feature 分支保持一致。

- [ ] **Step 2: 删除所有向 static 写回函数指针的 dispatch 函数**

调用点统一成：

```rust
unsafe { selected_memchr2()(needle1, needle2, beg, end) }
```

或相应 `selected_lines_fwd()/selected_lines_bwd()`。不得保留 `static mut`、`#[allow(static_mut_refs)]` 或每次 feature detection。

- [ ] **Step 3: 验证 core SIMD**

```bash
rg -n "static mut .*DISPATCH|static_mut_refs" crates/core/src/simd
cargo test -p edit-plus-core --lib simd::
cargo clippy -p edit-plus-core --lib -- -D warnings
```

Expected: `rg` 无输出；测试与 Clippy PASS。

- [ ] **Step 4: 提交**

```bash
git add crates/core/src/simd/memchr2.rs crates/core/src/simd/lines_fwd.rs crates/core/src/simd/lines_bwd.rs
git commit -m "fix(core): synchronize SIMD dispatch initialization"
```

### Task 3: 用 OnceLock 替换 stdext memset 分派

**Files:**
- Modify: `crates/stdext/src/simd/memset.rs:92-115`

- [ ] **Step 1: 替换函数指针 static mut**

```rust
type MemsetFn = unsafe fn(*mut u8, *mut u8, u64);

#[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "loongarch64"))]
static MEMSET_DISPATCH: std::sync::OnceLock<MemsetFn> = std::sync::OnceLock::new();

fn selected_memset() -> MemsetFn {
    *MEMSET_DISPATCH.get_or_init(|| {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        if is_x86_feature_detected!("avx2") { return memset_avx2; }
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        { return memset_sse2; }
        #[cfg(target_arch = "loongarch64")]
        { memset_fallback }
    })
}
```

public wrapper 只调用 `selected_memset()` 返回值；删除自修改 `memset_dispatch`。

- [ ] **Step 2: 验证并提交**

```bash
cargo test -p stdext --lib simd::
cargo clippy -p stdext --all-targets -- -D warnings
git add crates/stdext/src/simd/memset.rs
git commit -m "fix(stdext): synchronize memset dispatch initialization"
```

### Task 4: 收口 ICU 编码表与线程本地 handle

**Files:**
- Modify: `crates/core/src/icu.rs:145-205,343-485`

- [ ] **Step 1: 增加并发 ICU API 测试**

```rust
#[test]
fn safe_icu_apis_support_concurrent_first_use() {
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
    let handles: Vec<_> = (0..16)
        .map(|_| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let encodings = get_available_encodings();
                assert!(!encodings.all.is_empty());
                assert_eq!(compare_strings(b"file2", b"file10"), std::cmp::Ordering::Less);
                let arena = stdext::arena::Arena::new(4096).unwrap();
                let folded = fold_case(&arena, "Straße");
                assert_eq!(std::str::from_utf8(&folded).unwrap(), "strasse");
            })
        })
        .collect();
    for handle in handles { handle.join().unwrap(); }
}
```

- [ ] **Step 2: 编码表改为进程级 OnceLock**

```rust
static ENCODINGS: std::sync::OnceLock<Encodings> = std::sync::OnceLock::new();

pub fn get_available_encodings() -> &'static Encodings {
    ENCODINGS.get_or_init(build_available_encodings)
}
```

把原初始化体移入 `fn build_available_encodings() -> Encodings`；泄漏后的 slice 仍为 `'static`，但不再读写全局可变对象。

- [ ] **Step 3: Collator/CaseMap 改为 thread-local OnceCell**

```rust
thread_local! {
    static ROOT_COLLATOR: std::cell::OnceCell<*mut icu_ffi::UCollator> = const { std::cell::OnceCell::new() };
    static ROOT_CASEMAP: std::cell::OnceCell<*mut icu_ffi::UCaseMap> = const { std::cell::OnceCell::new() };
}
```

`compare_strings` 与 `fold_case` 在 `with` 闭包中调用 `get_or_init`。在 SAFETY 注释中明确：handle 只在创建它的线程使用；ICU 不可用时缓存 null 并走 ASCII/原字符串 fallback。不得为裸指针添加 `unsafe impl Sync`。

- [ ] **Step 4: 验证 static mut 清零和并发测试**

```bash
rg -n "static mut|static_mut_refs" crates/core/src/icu.rs crates/core/src/simd crates/stdext/src/simd
cargo test -p edit-plus-core --lib safe_icu_apis_support_concurrent_first_use -- --exact
cargo test -p edit-plus-core --lib
cargo test -p stdext --lib
```

Expected: `rg` 无输出；全部测试 PASS。

- [ ] **Step 5: 运行 Miri 可覆盖测试并提交**

```bash
rustup toolchain install nightly --component miri --profile minimal
cargo +nightly miri test -p stdext --lib simd::
git add crates/core/src/icu.rs
git commit -m "fix(core): make ICU lazy state thread-safe"
```

Expected: Miri PASS；Miri 不运行 platform ICU FFI 测试，只覆盖纯 Rust/stdext 路径。
