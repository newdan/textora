//! Notora 打开和保存请求的产品类型。

use std::path::PathBuf;

use appkit_core::tab_set::types::TabId;

/// 外部打开来源最终统一为同一个 effect；路径来源不应拥有单独的验证逻辑。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalOpenRequest {
    ShowFileDialog,
    Paths(Vec<PathBuf>),
}

/// 用户显式保存当前文档时已经由产品判定好的来源类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManualSaveRequest {
    Note { tab_id: TabId, content_revision: u64 },
    ExistingExternalFile { tab_id: TabId },
    UntitledExternalFile { tab_id: TabId, external_file_id: notora_core::ExternalFileId },
}
