//! Platform-agnostic native menu bar abstraction.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{LazyLock, Mutex};

/// Cached recent file paths for menu building.
/// Index in the vec corresponds to the menu tag.
#[allow(dead_code)]
pub(crate) static RECENT_FILES: LazyLock<Mutex<Vec<PathBuf>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// Number of recent file slots in the menu.
const RECENT_SLOTS: usize = 20;
/// Base tag for recent file menu items (100-119).
const RECENT_TAG_BASE: isize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    About,
    Preferences,
    Quit,
    NewFile,
    OpenFile,
    Save,
    SaveAs,
    CloseTab,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Find,
    ToggleTabBar,
    ToggleStatusBar,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    /// Open recent file at the given index (0-based).
    OpenRecentFile(usize),
    /// Clear all recent file history.
    ClearRecentFiles,
    SetThemeModeSystem,
    SetThemeModeDark,
    SetThemeModeLight,
    ToggleLineNumbers,
    ToggleWordWrap,
    SetViewModeSidebar,
    SetViewModeTabs,
}

pub struct NativeMenu {
    rx: Option<mpsc::Receiver<MenuAction>>,
}

impl NativeMenu {
    pub fn build(recent_files: &[PathBuf]) -> Self {
        #[cfg(target_os = "macos")]
        {
            macos::build_native_menu(Some(recent_files))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = recent_files;
            NativeMenu { rx: None }
        }
    }

    /// Build the menu bar before background recent-file validation completes.
    pub fn build_loading() -> Self {
        #[cfg(target_os = "macos")]
        {
            macos::build_native_menu(None)
        }
        #[cfg(not(target_os = "macos"))]
        {
            NativeMenu { rx: None }
        }
    }

    pub fn poll_action(&self) -> Option<MenuAction> {
        self.rx.as_ref()?.try_recv().ok()
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{MenuAction, RECENT_SLOTS, RECENT_TAG_BASE};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::mpsc;

    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{AnyThread, MainThreadOnly, define_class, sel};
    use objc2_app_kit::{NSMenu, NSMenuItem};
    use objc2_foundation::{MainThreadMarker, NSString};

    static MENU_TX: Mutex<Option<mpsc::Sender<MenuAction>>> = Mutex::new(None);
    static MENU_TARGET: Mutex<Option<Retained<MenuTarget>>> = Mutex::new(None);

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "EditPlusMenuTarget"]
        struct MenuTarget;

        impl MenuTarget {
            #[unsafe(method(menuAction:))]
            fn menu_action(&self, sender: &objc2::runtime::AnyObject) {
                let tag: isize = unsafe { objc2::msg_send![sender, tag] };
                let action = match tag {
                    1 => MenuAction::About,
                    2 => MenuAction::Preferences,
                    3 => MenuAction::Quit,
                    4 => MenuAction::NewFile,
                    5 => MenuAction::OpenFile,
                    6 => MenuAction::Save,
                    7 => MenuAction::SaveAs,
                    8 => MenuAction::CloseTab,
                    9 => MenuAction::Undo,
                    10 => MenuAction::Redo,
                    11 => MenuAction::Cut,
                    12 => MenuAction::Copy,
                    13 => MenuAction::Paste,
                    14 => MenuAction::SelectAll,
                    15 => MenuAction::Find,
                    16 => MenuAction::ToggleTabBar,
                    17 => MenuAction::ToggleStatusBar,
                    18 => MenuAction::ZoomIn,
                    19 => MenuAction::ZoomOut,
                    20 => MenuAction::ZoomReset,
                    21 => MenuAction::ClearRecentFiles,
                    22 => MenuAction::SetThemeModeSystem,
                    23 => MenuAction::SetThemeModeDark,
                    24 => MenuAction::SetThemeModeLight,
                    25 => MenuAction::ToggleLineNumbers,
                    26 => MenuAction::ToggleWordWrap,
                    27 => MenuAction::SetViewModeSidebar,
                    28 => MenuAction::SetViewModeTabs,
                    t if t >= RECENT_TAG_BASE && t < RECENT_TAG_BASE + RECENT_SLOTS as isize => {
                        MenuAction::OpenRecentFile((t - RECENT_TAG_BASE) as usize)
                    }
                    _ => return,
                };
                if let Ok(guard) = MENU_TX.lock()
                    && let Some(ref tx) = *guard {
                        let _ = tx.send(action);
                    }
            }
        }
    );

    fn make_item(
        title: &str,
        tag: isize,
        key_equiv: &str,
        target: &MenuTarget,
        mtm: MainThreadMarker,
    ) -> Retained<NSMenuItem> {
        unsafe {
            let t = NSString::from_str(title);
            let k = NSString::from_str(key_equiv);
            let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &t,
                Some(sel!(menuAction:)),
                &k,
            );
            item.setTag(tag);
            item.setTarget(Some(target));
            item
        }
    }

    fn make_submenu(title: &str, sub: &NSMenu, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
        unsafe {
            let t = NSString::from_str(title);
            let k = NSString::from_str("");
            let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &t,
                None,
                &k,
            );
            item.setSubmenu(Some(sub));
            item
        }
    }

    fn separator(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
        NSMenuItem::separatorItem(mtm)
    }

    fn new_menu(title: &str, mtm: MainThreadMarker) -> Retained<NSMenu> {
        NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title))
    }

    fn add_disabled_recent_item(
        menu: &NSMenu,
        title: &str,
        target: &MenuTarget,
        mtm: MainThreadMarker,
    ) {
        let item = make_item(title, RECENT_TAG_BASE, "", target, mtm);
        item.setEnabled(false);
        menu.addItem(&item);
    }

    fn build_menu_items(
        main_menu: &NSMenu,
        target: &MenuTarget,
        mtm: MainThreadMarker,
        recent_files: Option<&[PathBuf]>,
    ) {
        // -- App menu --
        {
            let m = new_menu("", mtm);
            m.addItem(&make_item("关于 edit+", 1, "", target, mtm));
            m.addItem(&separator(mtm));
            // Settings submenu
            {
                let settings = new_menu("设置", mtm);
                let theme = new_menu("主题", mtm);
                theme.addItem(&make_item("跟随系统", 22, "", target, mtm));
                theme.addItem(&make_item("深色模式", 23, "", target, mtm));
                theme.addItem(&make_item("浅色模式", 24, "", target, mtm));
                settings.addItem(&make_submenu("主题", &theme, mtm));
                let view_mode = new_menu("视图模式", mtm);
                view_mode.addItem(&make_item("Sidebar 模式", 27, "", target, mtm));
                view_mode.addItem(&make_item("Tabs 模式", 28, "", target, mtm));
                settings.addItem(&make_submenu("视图模式", &view_mode, mtm));
                settings.addItem(&make_item("显示行号", 25, "", target, mtm));
                settings.addItem(&make_item("自动换行", 26, "", target, mtm));
                m.addItem(&make_submenu("设置", &settings, mtm));
            }
            m.addItem(&make_item("偏好设置…", 2, ",", target, mtm));
            m.addItem(&separator(mtm));
            m.addItem(&make_item("退出 edit+", 3, "q", target, mtm));
            main_menu.addItem(&make_submenu("", &m, mtm));
        }
        // -- File --
        {
            let m = new_menu("文件", mtm);
            m.addItem(&make_item("新建", 4, "n", target, mtm));
            m.addItem(&make_item("打开…", 5, "o", target, mtm));

            // Open Recent submenu remains available while background validation runs.
            let recent = new_menu("打开最近的文件", mtm);
            match recent_files {
                None => add_disabled_recent_item(&recent, "正在加载最近文件…", target, mtm),
                Some([]) => add_disabled_recent_item(&recent, "没有最近文件", target, mtm),
                Some(paths) => {
                    for (i, path) in paths.iter().enumerate().take(RECENT_SLOTS) {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.to_string_lossy().into_owned());
                        let tag = RECENT_TAG_BASE + i as isize;
                        recent.addItem(&make_item(&name, tag, "", target, mtm));
                    }
                    recent.addItem(&separator(mtm));
                    recent.addItem(&make_item("清除最近文件", 21, "", target, mtm));
                }
            }
            m.addItem(&make_submenu("打开最近的文件", &recent, mtm));

            m.addItem(&separator(mtm));
            m.addItem(&make_item("保存", 6, "s", target, mtm));
            m.addItem(&make_item("另存为…", 7, "S", target, mtm));
            m.addItem(&separator(mtm));
            m.addItem(&make_item("关闭标签页", 8, "w", target, mtm));
            main_menu.addItem(&make_submenu("文件", &m, mtm));
        }
        // -- Edit --
        {
            let m = new_menu("编辑", mtm);
            m.addItem(&make_item("撤销", 9, "z", target, mtm));
            m.addItem(&make_item("重做", 10, "Z", target, mtm));
            m.addItem(&separator(mtm));
            m.addItem(&make_item("剪切", 11, "x", target, mtm));
            m.addItem(&make_item("复制", 12, "c", target, mtm));
            m.addItem(&make_item("粘贴", 13, "v", target, mtm));
            m.addItem(&make_item("全选", 14, "a", target, mtm));
            m.addItem(&separator(mtm));
            m.addItem(&make_item("查找", 15, "f", target, mtm));
            main_menu.addItem(&make_submenu("编辑", &m, mtm));
        }
        // -- View --
        {
            let m = new_menu("视图", mtm);
            m.addItem(&make_item("显示/隐藏标签栏", 16, "", target, mtm));
            m.addItem(&make_item("显示/隐藏状态栏", 17, "", target, mtm));
            m.addItem(&separator(mtm));
            m.addItem(&make_item("放大", 18, "=", target, mtm));
            m.addItem(&make_item("缩小", 19, "-", target, mtm));
            m.addItem(&make_item("重置缩放", 20, "0", target, mtm));
            main_menu.addItem(&make_submenu("视图", &m, mtm));
        }
    }

    pub(crate) fn build_native_menu(recent_files: Option<&[PathBuf]>) -> super::NativeMenu {
        if let Ok(mut guard) = super::RECENT_FILES.lock() {
            guard.clear();
            if let Some(paths) = recent_files {
                guard.extend_from_slice(paths);
            }
        }

        let (tx, rx) = mpsc::channel::<MenuAction>();
        {
            let mut guard = MENU_TX.lock().unwrap();
            *guard = Some(tx);
        }

        let mtm = match MainThreadMarker::new() {
            Some(m) => m,
            None => {
                eprintln!("native_menu: not on main thread, skipping");
                return super::NativeMenu { rx: Some(rx) };
            }
        };

        let target: Retained<MenuTarget> = unsafe {
            let alloc = MenuTarget::alloc();
            let alloc_nsobject: objc2::rc::Allocated<NSObject> = std::mem::transmute::<
                objc2::rc::Allocated<MenuTarget>,
                objc2::rc::Allocated<NSObject>,
            >(alloc);
            let initialized: Retained<MenuTarget> = std::mem::transmute::<
                Retained<NSObject>,
                Retained<MenuTarget>,
            >(NSObject::init(alloc_nsobject));
            initialized
        };

        let main_menu = new_menu("", mtm);
        build_menu_items(&main_menu, &target, mtm, recent_files);

        let app = objc2_app_kit::NSApp(mtm);
        app.setMainMenu(Some(&main_menu));

        if let Ok(mut guard) = MENU_TARGET.lock() {
            *guard = Some(target);
        }

        super::NativeMenu { rx: Some(rx) }
    }
}
