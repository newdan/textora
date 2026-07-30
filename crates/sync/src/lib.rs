//! 独立的 Syncthing 控制面适配器。
//!
//! 该 crate 只依赖 Syncthing 的 HTTP API 和领域类型，不依赖 Textora 的
//! app、UI 或文档状态。

#![forbid(unsafe_code)]

mod client;
mod dto;
mod endpoint;
mod error;
mod identifiers;
mod ignore;
mod service;
mod state;

pub use client::SyncthingClient;
pub use dto::{
    ConfigurationDifference, DeviceConfig, FolderConfig, FolderPhase, FolderStatus, InstanceInfo,
    PendingDevice, PendingFolder, RemoteDeviceSpec, SYNCTHING_DYNAMIC_ADDRESS, StaticSyncAddress,
    compare_device_configuration, compare_folder_configuration,
};
pub use endpoint::LoopbackEndpoint;
pub use error::SyncError;
pub use identifiers::{ApiKey, DeviceId, FolderId};
pub use ignore::{
    ManagedIgnoreState, TEXTORA_MANAGED_BEGIN, TEXTORA_MANAGED_END, TEXTORA_MANAGED_RULE,
    append_managed_ignore_block, inspect_managed_ignore_rules, repair_managed_ignore_block,
};
pub use service::{SyncCommand, SyncResult, SyncService};
pub use state::{
    BackoffAction, ConnectionObservation, EventBatch, EventCursor, LibraryObservation,
    LibrarySyncState, RemoteEvent, RetryBackoff, RetryFailure, SyncEvent, SyncEventKind,
    reduce_event_batch, reduce_library_state,
};
