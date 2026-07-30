# 大文件拆分与架构优化计划

## 目标
当前项目中有几个极为庞大的核心文件：
1. `crates/app/src/app.rs` (3560行) - 上帝对象，包含生命周期、窗口、事件、Tab、渲染状态等。
2. `crates/core/src/buffer/text_buffer.rs` (3541行) - 包含文本、IO、撤销重做、搜索、编辑逻辑等。
3. `crates/app/src/render_pipeline.rs` (2004行) - 巨型函数 `shape_visible_lines`。

本计划旨在将这三个巨型文件按功能原子化拆分，降低耦合度，提升可维护性。每个阶段独立提交，确保随时可编译运行。

---

## 阶段 1：拆分 `text_buffer.rs` (基础层)

`TextBuffer` 应作为外观(Facade)，内部模块聚合。
- **阶段 1.1：提取历史与撤销模块**
  - **内容**：新建 `crates/core/src/buffer/history.rs`。将 `HistoryEntry`, `HistoryType`, `ActiveEditGroupInfo` 及撤销重做栈、操作合并逻辑迁移。
  - **接口**：定义 `HistoryManager` 结构体，提供 `push`, `undo`, `redo` 方法。
- **阶段 1.2：提取搜索模块**
  - **内容**：新建 `crates/core/src/buffer/search.rs`。将 `ActiveSearch`, `SearchOptions` 及正则替换/搜索逻辑分离。
  - **接口**：定义 `SearchContext` 管理 ICU 和正则状态。
- **阶段 1.3：提取文件 IO 模块**
  - **内容**：新建 `crates/core/src/buffer/io.rs`。迁移 `read_file`, `save_as_string`, BOM 处理, CRLF/LF 转换。
- **阶段 1.4：提取光标与编辑命令**
  - **内容**：新建 `crates/core/src/buffer/edit.rs`，承载纯文本插入、删除、缩进等具体修改动作。并将基于字符、单词的移动合并至 `navigation.rs` 或独立。

---

## 阶段 2：拆分 `app.rs` (核心调度层)

`App` 应当只做顶层生命周期与事件捕获，业务逻辑下发。
- **阶段 2.1：提取 Tab 与工作区管理**
  - **内容**：新建 `crates/app/src/workspace.rs`。聚合 `doc_views`, `active_index`, `pinned_indices`, `tab_history` 以及后退/前进导航、文件打开等。
  - **接口**：设计 `Workspace` 结构体，封装对 `DocumentView` 的所有跨标签操作。
- **阶段 2.2：提取菜单动作处理器**
  - **内容**：新建 `crates/app/src/menu_handler.rs`。剥离 `dispatch_menu_action` 和 `execute_context_menu_action`。
- **阶段 2.3：提取渲染资源状态**
  - **内容**：新建 `crates/app/src/render_state.rs`。剥离 `GpuState`, `TextState` 及初始化流程，统一管理 wgpu 资源。
- **阶段 2.4：拆分事件处理循环**
  - **内容**：目前的 `winit` 键盘/鼠标事件解析过于长。按事件类别抽取 `handle_mouse_event`, `handle_keyboard_event`，放入专门的模块。

---

## 阶段 3：拆分 `render_pipeline.rs` (渲染层)

重点突破 `shape_visible_lines` 这个千行函数。
- **阶段 3.1：提取行号与 Gutter 渲染**
  - **内容**：抽取独立的 `generate_line_number_vertices` 及边距计算到 `crates/app/src/gutter.rs`。
- **阶段 3.2：分离排版引擎调度 (Shaping)**
  - **内容**：将 cosmic-text 的调用、word-wrap 计算及 RenderCache 存取单独包装到 `layout` 逻辑层，避免与 WGPU 顶点生成混杂。
- **阶段 3.3：提取 UI 装饰渲染**
  - **内容**：将 `cursor_vertices` (光标), `selection_vertices` (选中高亮) 转移到 `crates/app/src/decorations.rs` 独立维护。

---

## 执行协议
1. 每次仅处理一个子阶段。
2. 每个模块拆出后，补齐或平移对应的测试代码。
3. 测试通过且保证 `cargo check` 无错误后，方可进行下一个子阶段。
4. 在涉及结构体跨文件引用的情况下，优先理清 struct 的可见性 (pub(crate)) 和生命周期，防止所有权纠缠。
