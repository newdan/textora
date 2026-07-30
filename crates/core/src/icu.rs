//! Bindings to the ICU library.

use std::cmp::Ordering;
use std::ffi::{CStr, c_char};
use std::mem::MaybeUninit;
use std::ptr::{null, null_mut};
use std::sync::OnceLock;
use std::{fmt, mem};

use stdext::arena::{Arena, scratch_arena};
use stdext::arena_format;
use stdext::collections::{BString, BVec};

mod sys {
    use std::ffi::{CStr, c_char, c_void};
    use std::io;
    use std::ptr::NonNull;

    pub struct LibIcu {
        pub libicuuc: NonNull<std::ffi::c_void>,
        pub libicui18n: NonNull<std::ffi::c_void>,
    }

    #[cfg(unix)]
    pub fn load_icu() -> io::Result<LibIcu> {
        unsafe {
            const LIBICUUC: &CStr = c"libicucore.dylib";
            const LIBICUI18N: &CStr = c"libicucore.dylib";

            let libicuuc = load_library(LIBICUUC)?;
            let libicui18n = load_library(LIBICUI18N)?;
            Ok(LibIcu { libicuuc, libicui18n })
        }
    }

    #[cfg(not(unix))]
    pub fn load_icu() -> io::Result<LibIcu> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "ICU loading not implemented for this platform",
        ))
    }

    #[cfg(unix)]
    unsafe fn load_library(name: &CStr) -> io::Result<NonNull<std::ffi::c_void>> {
        unsafe {
            let handle = libc::dlopen(name.as_ptr(), libc::RTLD_LAZY);
            NonNull::new(handle).ok_or_else(io::Error::last_os_error)
        }
    }

    /// Loads a function from a dynamic library.
    ///
    /// # Safety
    ///
    /// This function is highly unsafe as it requires you to know the exact type
    /// of the function you're loading. No type checks whatsoever are performed.
    #[cfg(unix)]
    pub unsafe fn get_proc_address<T>(
        handle: NonNull<c_void>,
        name: *const c_char,
    ) -> io::Result<T> {
        unsafe {
            let sym = libc::dlsym(handle.as_ptr(), name);
            if sym.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(std::mem::transmute_copy(&sym))
            }
        }
    }

    /// Detect ICU symbol renaming suffix (e.g., version suffix on Linux).
    /// Returns the suffix string if the library uses renamed symbols.
    ///
    /// **Platform note**: Only relevant on Linux where distro ICU packages
    /// version-suffix exported symbols (e.g., `u_init_67`).  On macOS the
    /// system `libicucore.dylib` does not rename symbols so this is never
    /// called.  Marked `todo!()` — Linux ICU is not yet supported.
    #[cfg(edit_icu_renaming_auto_detect)]
    pub fn icu_detect_renaming_suffix(
        _arena: &stdext::arena::Arena,
        _lib: NonNull<c_void>,
    ) -> Option<*const c_char> {
        todo!(
            "M8/N4: copy from edit/sys/unix.rs when Linux support is needed; see plans.md stage 11"
        )
    }

    /// Append an ICU renaming suffix to a symbol name.
    ///
    /// **Platform note**: Linux only — see `icu_detect_renaming_suffix`.
    #[cfg(edit_icu_renaming_auto_detect)]
    pub fn icu_add_renaming_suffix<'a>(
        _arena: &stdext::arena::Arena<'a>,
        name: *const c_char,
        _suffix: &*const c_char,
    ) -> *const c_char {
        todo!(
            "M8/N4: copy from edit/sys/unix.rs when Linux support is needed; see plans.md stage 11"
        )
    }
}

#[allow(dead_code)]
pub(crate) const ILLEGAL_ARGUMENT_ERROR: Error = Error(1); // U_ILLEGAL_ARGUMENT_ERROR
pub const ICU_MISSING_ERROR: Error = Error(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error(u32);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn format(code: u32) -> &'static str {
            let Ok(f) = init_if_needed() else {
                return "";
            };

            let status = icu_ffi::UErrorCode::new(code);
            let ptr = unsafe { (f.u_errorName)(status) };
            if ptr.is_null() {
                return "";
            }

            let str = unsafe { CStr::from_ptr(ptr) };
            str.to_str().unwrap_or("")
        }

        let code = self.0;
        if code != 0
            && let msg = format(code)
            && !msg.is_empty()
        {
            write!(f, "ICU Error: {msg}")
        } else {
            write!(f, "ICU Error: {code:#08x}")
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy)]
pub struct Encoding {
    pub label: &'static str,
    pub canonical: &'static str,
}

pub struct Encodings {
    pub preferred: &'static [Encoding],
    pub all: &'static [Encoding],
}

static ENCODINGS: OnceLock<Encodings> = OnceLock::new();

/// Returns a list of encodings ICU supports.
pub fn get_available_encodings() -> &'static Encodings {
    ENCODINGS.get_or_init(|| {
        let scratch = scratch_arena(None);
        let mut preferred = BVec::empty();
        let mut alternative = BVec::empty();

        // These encodings are always available.
        preferred.push(&*scratch, Encoding { label: "UTF-8", canonical: "UTF-8" });
        preferred.push(&*scratch, Encoding { label: "UTF-8 BOM", canonical: "UTF-8 BOM" });

        if let Ok(f) = init_if_needed() {
            let mut n = 0;
            loop {
                let name = unsafe { (f.ucnv_getAvailableName)(n) };
                if name.is_null() {
                    break;
                }

                n += 1;

                let name = unsafe { CStr::from_ptr(name).to_str().unwrap_unchecked() };
                // We have already pushed UTF-8 above and can skip it.
                // There is no need to filter UTF-8 BOM here,
                // since ICU does not distinguish it from UTF-8.
                if name.is_empty() || name == "UTF-8" {
                    continue;
                }

                let mut status = icu_ffi::U_ZERO_ERROR;
                let mime = unsafe {
                    (f.ucnv_getStandardName)(name.as_ptr(), c"MIME".as_ptr().cast(), &mut status)
                };
                if !mime.is_null() && status.is_success() {
                    let mime = unsafe { CStr::from_ptr(mime).to_str().unwrap_unchecked() };
                    preferred.push(&*scratch, Encoding { label: mime, canonical: name });
                } else {
                    alternative.push(&*scratch, Encoding { label: name, canonical: name });
                }
            }
        }

        let preferred_len = preferred.len();

        // Combine the preferred and alternative encodings into a single list.
        let mut all = Vec::with_capacity(preferred.len() + alternative.len());
        all.extend(preferred);
        all.extend(alternative);

        let all = all.leak();
        Encodings { preferred: &all[..preferred_len], all: &all[..] }
    })
}

/// Converts between two encodings using ICU.
pub struct Converter<'pivot> {
    source: *mut icu_ffi::UConverter,
    target: *mut icu_ffi::UConverter,
    pivot_buffer: &'pivot mut [MaybeUninit<u16>],
    pivot_source: *mut u16,
    pivot_target: *mut u16,
    reset: bool,
}

impl Drop for Converter<'_> {
    fn drop(&mut self) {
        let f = assume_loaded();
        unsafe { (f.ucnv_close)(self.source) };
        unsafe { (f.ucnv_close)(self.target) };
    }
}

impl<'pivot> Converter<'pivot> {
    /// Constructs a new `Converter` instance.
    ///
    /// # Parameters
    ///
    /// * `pivot_buffer`: A buffer used to cache partial conversions.
    ///   Don't make it too small.
    /// * `source_encoding`: The source encoding name (e.g., "UTF-8").
    /// * `target_encoding`: The target encoding name (e.g., "UTF-16").
    pub fn new(
        pivot_buffer: &'pivot mut [MaybeUninit<u16>],
        source_encoding: &str,
        target_encoding: &str,
    ) -> Result<Self> {
        let f = init_if_needed()?;

        let arena = scratch_arena(None);
        let source_encoding = Self::append_nul(&arena, source_encoding);
        let target_encoding = Self::append_nul(&arena, target_encoding);

        let mut status = icu_ffi::U_ZERO_ERROR;
        let source = unsafe { (f.ucnv_open)(source_encoding.as_ptr(), &mut status) };
        let target = unsafe { (f.ucnv_open)(target_encoding.as_ptr(), &mut status) };
        if status.is_failure() {
            if !source.is_null() {
                unsafe { (f.ucnv_close)(source) };
            }
            if !target.is_null() {
                unsafe { (f.ucnv_close)(target) };
            }
            return Err(status.as_error());
        }

        let pivot_source = pivot_buffer.as_mut_ptr().cast::<u16>();
        let pivot_target = unsafe { pivot_source.add(pivot_buffer.len()) };

        Ok(Self { source, target, pivot_buffer, pivot_source, pivot_target, reset: true })
    }

    fn append_nul<'a>(arena: &'a Arena, input: &str) -> BString<'a> {
        arena_format!(arena, "{}\0", input)
    }

    /// Performs one step of the encoding conversion.
    ///
    /// # Parameters
    ///
    /// * `input`: The input buffer to convert from.
    ///   It should be in the `source_encoding` that was previously specified.
    /// * `output`: The output buffer to convert to.
    ///   It should be in the `target_encoding` that was previously specified.
    ///
    /// # Returns
    ///
    /// A tuple containing:
    /// 1. The number of bytes read from the input buffer.
    /// 2. The number of bytes written to the output buffer.
    pub fn convert(
        &mut self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
    ) -> Result<(usize, usize)> {
        let f = assume_loaded();

        let input_beg = input.as_ptr();
        let input_end = unsafe { input_beg.add(input.len()) };
        let mut input_ptr = input_beg;

        let output_beg = output.as_mut_ptr().cast::<u8>();
        let output_end = unsafe { output_beg.add(output.len()) };
        let mut output_ptr = output_beg;

        let pivot_beg = self.pivot_buffer.as_mut_ptr().cast::<u16>();
        let pivot_end = unsafe { pivot_beg.add(self.pivot_buffer.len()) };

        let flush = input.is_empty();
        let mut status = icu_ffi::U_ZERO_ERROR;

        unsafe {
            (f.ucnv_convertEx)(
                /* target_cnv   */ self.target,
                /* source_cnv   */ self.source,
                /* target       */ &mut output_ptr,
                /* target_limit */ output_end,
                /* source       */ &mut input_ptr,
                /* source_limit */ input_end,
                /* pivot_start  */ pivot_beg,
                /* pivot_source */ &mut self.pivot_source,
                /* pivot_target */ &mut self.pivot_target,
                /* pivot_limit  */ pivot_end,
                /* reset        */ self.reset,
                /* flush        */ flush,
                /* status       */ &mut status,
            );
        }

        self.reset = false;
        if status.is_failure() && status != icu_ffi::U_BUFFER_OVERFLOW_ERROR {
            return Err(status.as_error());
        }

        let input_advance = unsafe { input_ptr.offset_from(input_beg) as usize };
        let output_advance = unsafe { output_ptr.offset_from(output_beg) as usize };
        Ok((input_advance, output_advance))
    }
}

// In benchmarking, I found that the performance does not really change much by changing this value.
// I picked 64 because it seemed like a reasonable lower bound.
thread_local! {
    static ROOT_COLLATOR: std::cell::OnceCell<*mut icu_ffi::UCollator> = const { std::cell::OnceCell::new() };
    static ROOT_CASEMAP: std::cell::OnceCell<*mut icu_ffi::UCaseMap> = const { std::cell::OnceCell::new() };
}

pub fn compare_strings(a: &[u8], b: &[u8]) -> Ordering {
    let coll = ROOT_COLLATOR.with(|cell| {
        *cell.get_or_init(|| {
            let mut coll = null_mut();

            if let Ok(f) = init_if_needed() {
                let mut status = icu_ffi::U_ZERO_ERROR;
                coll = unsafe { (f.ucol_open)(c"".as_ptr(), &mut status) };
                // Turns on Unicode normalization. I'm not 100% sure if it's needed, but it only has a
                // small-ish performance impact and sounds like it's required for correct filename sorting.
                unsafe {
                    (f.ucol_setAttribute)(
                        coll,
                        icu_ffi::UCOL_NORMALIZATION_MODE,
                        icu_ffi::UCOL_ON,
                        &mut status,
                    );
                    // Ensure that "file2" < "file10", even though '2' > '1'.
                    // NOTE: This has a _huge_ performance impact. It's roughly 5x slower for our purpose of
                    // sorting filenames. If it becomes an issue, we could use `ucol_getSortKey` (only +25%).
                    // (`ucol_strcollUTF8` is faster if `UCOL_NUMERIC_COLLATION` isn't used.)
                    (f.ucol_setAttribute)(
                        coll,
                        icu_ffi::UCOL_NUMERIC_COLLATION,
                        icu_ffi::UCOL_ON,
                        &mut status,
                    );
                }
                if status.is_failure() {
                    coll = null_mut();
                }
            }

            coll
        })
    });

    if coll.is_null() {
        compare_strings_ascii(a, b)
    } else {
        let f = assume_loaded();
        let mut status = icu_ffi::U_ZERO_ERROR;
        let res = unsafe {
            (f.ucol_strcollUTF8)(
                coll,
                a.as_ptr(),
                a.len() as i32,
                b.as_ptr(),
                b.len() as i32,
                &mut status,
            )
        };

        match res {
            icu_ffi::UCollationResult::UCOL_EQUAL => Ordering::Equal,
            icu_ffi::UCollationResult::UCOL_GREATER => Ordering::Greater,
            icu_ffi::UCollationResult::UCOL_LESS => Ordering::Less,
        }
    }
}

/// Unicode collation via `ucol_strcollUTF8`, now for ASCII!
fn compare_strings_ascii(a: &[u8], b: &[u8]) -> Ordering {
    let mut iter = a.iter().zip(b.iter());

    // Low weight: Find the first character which differs.
    //
    // Remember that result in case all remaining characters are
    // case-insensitive equal, because then we use that as a fallback.
    while let Some((&a, &b)) = iter.next() {
        if a != b {
            let la = a.to_ascii_lowercase();
            let lb = b.to_ascii_lowercase();
            let mut order = la.cmp(&lb);

            if order == Ordering::Equal {
                // High weight: Find the first character which differs case-insensitively.
                // Otherwise, it falls back to (or rather: defaults to) a case-sensitive comparison.
                order = a.cmp(&b);

                for (a, b) in iter {
                    let la = a.to_ascii_lowercase();
                    let lb = b.to_ascii_lowercase();

                    if la != lb {
                        order = la.cmp(&lb);
                        break;
                    }
                }
            }

            return order;
        }
    }

    // Fallback: The shorter string wins.
    a.len().cmp(&b.len())
}

/// Converts the given UTF-8 string to lower case.
///
/// Case folding differs from lower case in that the output is primarily useful
/// to machines for comparisons. It's like applying Unicode normalization.
pub fn fold_case<'a>(arena: &'a Arena, input: &str) -> BString<'a> {
    let casemap = ROOT_CASEMAP.with(|cell| {
        *cell.get_or_init(|| {
            if let Ok(f) = init_if_needed() {
                let mut status = icu_ffi::U_ZERO_ERROR;
                unsafe { (f.ucasemap_open)(null(), 0, &mut status) }
            } else {
                null_mut()
            }
        })
    });

    if !casemap.is_null() {
        let f = assume_loaded();
        let mut status = icu_ffi::U_ZERO_ERROR;
        let mut output = BVec::empty();
        let mut output_len;

        // First, guess the output length:
        // TODO: What's a good heuristic here?
        {
            output.reserve_exact(arena, input.len() + 16);
            let output = output.spare_capacity_mut();
            output_len = unsafe {
                (f.ucasemap_utf8FoldCase)(
                    casemap,
                    output.as_mut_ptr().cast(),
                    output.len() as i32,
                    input.as_ptr().cast(),
                    input.len() as i32,
                    &mut status,
                )
            };
        }

        // If that failed to fit, retry with the correct length.
        if status == icu_ffi::U_BUFFER_OVERFLOW_ERROR && output_len > 0 {
            output.reserve_exact(arena, output_len as usize);
            let output = output.spare_capacity_mut();
            output_len = unsafe {
                (f.ucasemap_utf8FoldCase)(
                    casemap,
                    output.as_mut_ptr().cast(),
                    output.len() as i32,
                    input.as_ptr().cast(),
                    input.len() as i32,
                    &mut status,
                )
            };
        }

        if status.is_success() && output_len > 0 {
            unsafe {
                output.set_len(output_len as usize);
            }
            return unsafe { BString::from_utf8_unchecked(output) };
        }
    }

    let mut result = BString::from_str(arena, input);
    for b in unsafe { result.as_bytes_mut() } {
        b.make_ascii_lowercase();
    }
    result
}

// NOTE:
// To keep this neat, fields are ordered by prefix (= `ucol_` before `uregex_`),
// followed by functions in this order:
// * Static methods (e.g. `ucnv_getAvailableName`)
// * Constructors (e.g. `ucnv_open`)
// * Destructors (e.g. `ucnv_close`)
// * Methods, grouped by relationship
//   (e.g. `uregex_start64` and `uregex_end64` are near each other)
//
// WARNING:
// The order of the fields MUST match the order of strings in the following two arrays.
#[allow(non_snake_case)]
#[repr(C)]
struct LibraryFunctions {
    // LIBICUUC_PROC_NAMES
    u_errorName: icu_ffi::u_errorName,
    ucasemap_open: icu_ffi::ucasemap_open,
    ucasemap_utf8FoldCase: icu_ffi::ucasemap_utf8FoldCase,
    ucnv_getAvailableName: icu_ffi::ucnv_getAvailableName,
    ucnv_getStandardName: icu_ffi::ucnv_getStandardName,
    ucnv_open: icu_ffi::ucnv_open,
    ucnv_close: icu_ffi::ucnv_close,
    ucnv_convertEx: icu_ffi::ucnv_convertEx,
    utext_setup: icu_ffi::utext_setup,
    utext_close: icu_ffi::utext_close,

    // LIBICUI18N_PROC_NAMES
    ucol_open: icu_ffi::ucol_open,
    ucol_setAttribute: icu_ffi::ucol_setAttribute,
    ucol_strcollUTF8: icu_ffi::ucol_strcollUTF8,
    uregex_open: icu_ffi::uregex_open,
    uregex_close: icu_ffi::uregex_close,
    uregex_setTimeLimit: icu_ffi::uregex_setTimeLimit,
    uregex_setUText: icu_ffi::uregex_setUText,
    uregex_setText: icu_ffi::uregex_setText,
    uregex_reset64: icu_ffi::uregex_reset64,
    uregex_findNext: icu_ffi::uregex_findNext,
    uregex_groupCount: icu_ffi::uregex_groupCount,
    uregex_start64: icu_ffi::uregex_start64,
    uregex_end64: icu_ffi::uregex_end64,
}

macro_rules! proc_name {
    ($s:literal) => {
        concat!(env!("EDIT_CFG_ICU_EXPORT_PREFIX"), $s, env!("EDIT_CFG_ICU_EXPORT_SUFFIX"), "\0")
            .as_ptr()
            .cast()
    };
}

// Found in libicuuc.so on UNIX, icuuc.dll/icu.dll on Windows.
const LIBICUUC_PROC_NAMES: [*const c_char; 10] = [
    proc_name!("u_errorName"),
    proc_name!("ucasemap_open"),
    proc_name!("ucasemap_utf8FoldCase"),
    proc_name!("ucnv_getAvailableName"),
    proc_name!("ucnv_getStandardName"),
    proc_name!("ucnv_open"),
    proc_name!("ucnv_close"),
    proc_name!("ucnv_convertEx"),
    proc_name!("utext_setup"),
    proc_name!("utext_close"),
];

// Found in libicui18n.so on UNIX, icuin.dll/icu.dll on Windows.
const LIBICUI18N_PROC_NAMES: [*const c_char; 13] = [
    proc_name!("ucol_open"),
    proc_name!("ucol_setAttribute"),
    proc_name!("ucol_strcollUTF8"),
    proc_name!("uregex_open"),
    proc_name!("uregex_close"),
    proc_name!("uregex_setTimeLimit"),
    proc_name!("uregex_setUText"),
    proc_name!("uregex_setText"),
    proc_name!("uregex_reset64"),
    proc_name!("uregex_findNext"),
    proc_name!("uregex_groupCount"),
    proc_name!("uregex_start64"),
    proc_name!("uregex_end64"),
];

static LIBRARY_FUNCTIONS: OnceLock<Option<LibraryFunctions>> = OnceLock::new();

pub fn init() -> Result<()> {
    init_if_needed()?;
    Ok(())
}

fn init_if_needed() -> Result<&'static LibraryFunctions> {
    fn load() -> Option<LibraryFunctions> {
        unsafe {
            let Ok(icu) = sys::load_icu() else {
                return None;
            };

            type TransparentFunction = unsafe extern "C" fn() -> *const ();

            // OH NO I'M DOING A BAD THING
            //
            // If this assertion hits, you either forgot to update `LIBRARY_PROC_NAMES`
            // or you're on a platform where `dlsym` behaves different from classic UNIX and Windows.
            //
            // This code assumes that we can treat the `LibraryFunctions` struct containing various different function
            // pointers as an array of `TransparentFunction` pointers. In C, this works on any platform that supports
            // POSIX `dlsym` or equivalent, but I suspect Rust is once again being extra about it. In any case, that's
            // still better than loading every function one by one, just to blow up our binary size for no reason.
            const _: () = assert!(
                mem::size_of::<LibraryFunctions>()
                    == mem::size_of::<TransparentFunction>()
                        * (LIBICUUC_PROC_NAMES.len() + LIBICUI18N_PROC_NAMES.len())
            );

            let mut funcs = MaybeUninit::<LibraryFunctions>::uninit();
            let mut ptr = funcs.as_mut_ptr().cast::<TransparentFunction>();

            #[cfg(edit_icu_renaming_auto_detect)]
            let scratch_outer = scratch_arena(None);
            #[cfg(edit_icu_renaming_auto_detect)]
            let suffix = sys::icu_detect_renaming_suffix(&scratch_outer, icu.libicuuc);

            for (handle, names) in [
                (icu.libicuuc, &LIBICUUC_PROC_NAMES[..]),
                (icu.libicui18n, &LIBICUI18N_PROC_NAMES[..]),
            ] {
                for &name in names {
                    #[cfg(edit_icu_renaming_auto_detect)]
                    let scratch = scratch_arena(Some(&scratch_outer));
                    #[cfg(edit_icu_renaming_auto_detect)]
                    let name = sys::icu_add_renaming_suffix(&scratch, name, &suffix);

                    let Ok(func) = sys::get_proc_address(handle, name) else {
                        debug_assert!(
                            false,
                            "Failed to load ICU function: {:?}",
                            CStr::from_ptr(name)
                        );
                        return None;
                    };

                    ptr.write(func);
                    ptr = ptr.add(1);
                }
            }

            Some(funcs.assume_init())
        }
    }

    match LIBRARY_FUNCTIONS.get_or_init(load) {
        Some(f) => Ok(f),
        None => Err(ICU_MISSING_ERROR),
    }
}

fn assume_loaded() -> &'static LibraryFunctions {
    match LIBRARY_FUNCTIONS.get() {
        Some(Some(f)) => f,
        _ => panic!("ICU library not initialized; call init_if_needed() first"),
    }
}

mod icu_ffi {
    #![allow(dead_code, non_camel_case_types)]

    use std::ffi::{c_char, c_int, c_void};

    use super::Error;

    #[derive(Copy, Clone, Eq, PartialEq)]
    #[repr(transparent)]
    pub struct UErrorCode(c_int);

    impl UErrorCode {
        pub const fn new(code: u32) -> Self {
            Self(code as c_int)
        }

        pub fn is_success(&self) -> bool {
            self.0 <= 0
        }

        pub fn is_failure(&self) -> bool {
            self.0 > 0
        }

        pub fn as_error(&self) -> Error {
            debug_assert!(self.0 > 0);
            Error(self.0 as u32)
        }
    }

    pub const U_ZERO_ERROR: UErrorCode = UErrorCode(0);
    pub const U_BUFFER_OVERFLOW_ERROR: UErrorCode = UErrorCode(15);
    pub const U_UNSUPPORTED_ERROR: UErrorCode = UErrorCode(16);

    pub type u_errorName = unsafe extern "C" fn(code: UErrorCode) -> *const c_char;

    pub struct UConverter;

    pub type ucnv_getAvailableName = unsafe extern "C" fn(n: i32) -> *const c_char;

    pub type ucnv_getStandardName = unsafe extern "C" fn(
        name: *const u8,
        standard: *const u8,
        status: &mut UErrorCode,
    ) -> *const c_char;

    pub type ucnv_open =
        unsafe extern "C" fn(converter_name: *const u8, status: &mut UErrorCode) -> *mut UConverter;

    pub type ucnv_close = unsafe extern "C" fn(converter: *mut UConverter);

    pub type ucnv_convertEx = unsafe extern "C" fn(
        target_cnv: *mut UConverter,
        source_cnv: *mut UConverter,
        target: *mut *mut u8,
        target_limit: *const u8,
        source: *mut *const u8,
        source_limit: *const u8,
        pivot_start: *mut u16,
        pivot_source: *mut *mut u16,
        pivot_target: *mut *mut u16,
        pivot_limit: *const u16,
        reset: bool,
        flush: bool,
        status: &mut UErrorCode,
    );

    pub struct UCaseMap;

    pub type ucasemap_open = unsafe extern "C" fn(
        locale: *const c_char,
        options: u32,
        status: &mut UErrorCode,
    ) -> *mut UCaseMap;

    pub type ucasemap_utf8FoldCase = unsafe extern "C" fn(
        csm: *const UCaseMap,
        dest: *mut c_char,
        dest_capacity: i32,
        src: *const c_char,
        src_length: i32,
        status: &mut UErrorCode,
    ) -> i32;

    #[repr(C)]
    pub enum UCollationResult {
        UCOL_EQUAL = 0,
        UCOL_GREATER = 1,
        UCOL_LESS = -1,
    }

    #[repr(C)]
    pub struct UCollator;

    pub type ucol_open =
        unsafe extern "C" fn(loc: *const c_char, status: &mut UErrorCode) -> *mut UCollator;

    pub type ucol_setAttribute =
        unsafe extern "C" fn(coll: *mut UCollator, attr: i32, value: i32, status: &mut UErrorCode);

    pub const UCOL_NORMALIZATION_MODE: i32 = 4;
    pub const UCOL_NUMERIC_COLLATION: i32 = 7;
    pub const UCOL_ON: i32 = 17;

    pub type ucol_strcollUTF8 = unsafe extern "C" fn(
        coll: *mut UCollator,
        source: *const u8,
        source_length: i32,
        target: *const u8,
        target_length: i32,
        status: &mut UErrorCode,
    ) -> UCollationResult;

    // UText callback functions
    pub type UTextClone = unsafe extern "C" fn(
        dest: *mut UText,
        src: &UText,
        deep: bool,
        status: &mut UErrorCode,
    ) -> *mut UText;
    pub type UTextNativeLength = unsafe extern "C" fn(ut: &mut UText) -> i64;
    pub type UTextAccess =
        unsafe extern "C" fn(ut: &mut UText, native_index: i64, forward: bool) -> bool;
    pub type UTextExtract = unsafe extern "C" fn(
        ut: &mut UText,
        native_start: i64,
        native_limit: i64,
        dest: *mut u16,
        dest_capacity: i32,
        status: &mut UErrorCode,
    ) -> i32;
    pub type UTextReplace = unsafe extern "C" fn(
        ut: &mut UText,
        native_start: i64,
        native_limit: i64,
        replacement_text: *const u16,
        replacement_length: i32,
        status: &mut UErrorCode,
    ) -> i32;
    pub type UTextCopy = unsafe extern "C" fn(
        ut: &mut UText,
        native_start: i64,
        native_limit: i64,
        native_dest: i64,
        move_text: bool,
        status: &mut UErrorCode,
    );
    pub type UTextMapOffsetToNative = unsafe extern "C" fn(ut: &UText) -> i64;
    pub type UTextMapNativeIndexToUTF16 =
        unsafe extern "C" fn(ut: &UText, native_index: i64) -> i32;
    pub type UTextClose = unsafe extern "C" fn(ut: &mut UText);

    #[repr(C)]
    pub struct UTextFuncs {
        pub table_size: i32,
        pub reserved1: i32,
        pub reserved2: i32,
        pub reserved3: i32,
        pub clone: Option<UTextClone>,
        pub native_length: Option<UTextNativeLength>,
        pub access: Option<UTextAccess>,
        pub extract: Option<UTextExtract>,
        pub replace: Option<UTextReplace>,
        pub copy: Option<UTextCopy>,
        pub map_offset_to_native: Option<UTextMapOffsetToNative>,
        pub map_native_index_to_utf16: Option<UTextMapNativeIndexToUTF16>,
        pub close: Option<UTextClose>,
        pub spare1: Option<UTextClose>,
        pub spare2: Option<UTextClose>,
        pub spare3: Option<UTextClose>,
    }

    #[repr(C)]
    pub struct UText {
        pub magic: u32,
        pub flags: i32,
        pub provider_properties: i32,
        pub size_of_struct: i32,
        pub chunk_native_limit: i64,
        pub extra_size: i32,
        pub native_indexing_limit: i32,
        pub chunk_native_start: i64,
        pub chunk_offset: i32,
        pub chunk_length: i32,
        pub chunk_contents: *const u16,
        pub p_funcs: &'static UTextFuncs,
        pub p_extra: *mut c_void,
        pub context: *mut c_void,
        pub p: *mut c_void,
        pub q: *mut c_void,
        pub r: *mut c_void,
        pub priv_p: *mut c_void,
        pub a: i64,
        pub b: i32,
        pub c: i32,
        pub priv_a: i64,
        pub priv_b: i32,
        pub priv_c: i32,
    }

    pub const UTEXT_MAGIC: u32 = 0x345ad82c;
    pub const UTEXT_PROVIDER_LENGTH_IS_EXPENSIVE: i32 = 1;
    pub const UTEXT_PROVIDER_STABLE_CHUNKS: i32 = 2;
    pub const UTEXT_PROVIDER_WRITABLE: i32 = 3;
    pub const UTEXT_PROVIDER_HAS_META_DATA: i32 = 4;
    pub const UTEXT_PROVIDER_OWNS_TEXT: i32 = 5;

    pub type utext_setup = unsafe extern "C" fn(
        ut: *mut UText,
        extra_space: i32,
        status: &mut UErrorCode,
    ) -> *mut UText;
    pub type utext_close = unsafe extern "C" fn(ut: *mut UText) -> *mut UText;

    #[repr(C)]
    pub struct UParseError {
        pub line: i32,
        pub offset: i32,
        pub pre_context: [u16; 16],
        pub post_context: [u16; 16],
    }

    #[repr(C)]
    pub struct URegularExpression;

    pub const UREGEX_UNIX_LINES: i32 = 1;
    pub const UREGEX_CASE_INSENSITIVE: i32 = 2;
    pub const UREGEX_COMMENTS: i32 = 4;
    pub const UREGEX_MULTILINE: i32 = 8;
    pub const UREGEX_LITERAL: i32 = 16;
    pub const UREGEX_DOTALL: i32 = 32;
    pub const UREGEX_UWORD: i32 = 256;
    pub const UREGEX_ERROR_ON_UNKNOWN_ESCAPES: i32 = 512;

    pub type uregex_open = unsafe extern "C" fn(
        pattern: *const u16,
        pattern_length: i32,
        flags: i32,
        pe: Option<&mut UParseError>,
        status: &mut UErrorCode,
    ) -> *mut URegularExpression;
    pub type uregex_close = unsafe extern "C" fn(regexp: *mut URegularExpression);
    pub type uregex_setTimeLimit =
        unsafe extern "C" fn(regexp: *mut URegularExpression, limit: i32, status: &mut UErrorCode);
    pub type uregex_setUText = unsafe extern "C" fn(
        regexp: *mut URegularExpression,
        text: *mut UText,
        status: &mut UErrorCode,
    );
    pub type uregex_setText = unsafe extern "C" fn(
        regexp: *mut URegularExpression,
        text: *const u16,
        text_length: i32,
        status: &mut UErrorCode,
    );
    pub type uregex_reset64 =
        unsafe extern "C" fn(regexp: *mut URegularExpression, index: i64, status: &mut UErrorCode);
    pub type uregex_findNext =
        unsafe extern "C" fn(regexp: *mut URegularExpression, status: &mut UErrorCode) -> bool;
    pub type uregex_groupCount =
        unsafe extern "C" fn(regexp: *mut URegularExpression, status: &mut UErrorCode) -> i32;
    pub type uregex_start64 = unsafe extern "C" fn(
        regexp: *mut URegularExpression,
        group_num: i32,
        status: &mut UErrorCode,
    ) -> i64;
    pub type uregex_end64 = unsafe extern "C" fn(
        regexp: *mut URegularExpression,
        group_num: i32,
        status: &mut UErrorCode,
    ) -> i64;
}

fn build_utf16_mapping(utf8: &[u8]) -> Result<(Vec<u16>, Vec<usize>)> {
    let text = std::str::from_utf8(utf8).map_err(|_| Error(8))?;
    let mut utf16 = Vec::with_capacity(text.len());
    let mut utf16_to_byte = Vec::with_capacity(text.len() + 1);

    for (byte_offset, ch) in text.char_indices() {
        let mut encoded = [0; 2];
        let encoded = ch.encode_utf16(&mut encoded);
        utf16_to_byte.push(byte_offset);
        utf16.push(encoded[0]);
        if encoded.len() > 1 {
            utf16_to_byte.push(byte_offset);
            utf16.push(encoded[1]);
        }
    }
    utf16_to_byte.push(utf8.len());

    Ok((utf16, utf16_to_byte))
}

pub struct Text {
    pub(crate) utf8: Vec<u8>,
    pub(crate) utf16: Vec<u16>,
    pub(crate) utf16_to_byte: Vec<usize>,
}

unsafe impl Send for Text {}

impl Text {
    /// # Safety
    /// ICU must have been initialized before this text is used by a regex.
    pub unsafe fn new(bytes: &[u8]) -> Result<Self> {
        let _ = init_if_needed()?;
        let (utf16, utf16_to_byte) = build_utf16_mapping(bytes)?;
        Ok(Self { utf8: bytes.to_vec(), utf16, utf16_to_byte })
    }

    pub fn rebuild(&mut self, bytes: &[u8]) -> Result<()> {
        let (utf16, utf16_to_byte) = build_utf16_mapping(bytes)?;
        self.utf8 = bytes.to_vec();
        self.utf16 = utf16;
        self.utf16_to_byte = utf16_to_byte;
        Ok(())
    }
}

pub struct Regex {
    re: *mut icu_ffi::URegularExpression,
    utf16_to_byte: Vec<usize>,
    byte_len: usize,
}

unsafe impl Send for Regex {}

impl Regex {
    pub const CASE_INSENSITIVE: u32 = 1;
    pub const LITERAL: u32 = 2;
    pub const MULTILINE: u32 = 4;

    /// # Safety
    /// ICU must have been initialized and `text` must outlive this call.
    pub unsafe fn new(pattern: &str, flags: u32, text: &Text) -> Result<Self> {
        let f = init_if_needed()?;
        let mut icu_flags = 0;
        if flags & Self::CASE_INSENSITIVE != 0 {
            icu_flags |= icu_ffi::UREGEX_CASE_INSENSITIVE;
        }
        if flags & Self::LITERAL != 0 {
            icu_flags |= icu_ffi::UREGEX_LITERAL;
        }
        if flags & Self::MULTILINE != 0 {
            icu_flags |= icu_ffi::UREGEX_MULTILINE;
        }

        let pattern_utf16: Vec<u16> = pattern.encode_utf16().collect();
        let mut status = icu_ffi::U_ZERO_ERROR;
        let mut parse_error = MaybeUninit::<icu_ffi::UParseError>::zeroed();
        let re = unsafe {
            (f.uregex_open)(
                pattern_utf16.as_ptr(),
                pattern_utf16.len() as i32,
                icu_flags,
                Some(&mut *parse_error.as_mut_ptr()),
                &mut status,
            )
        };
        if status.is_failure() {
            if !re.is_null() {
                unsafe { (f.uregex_close)(re) };
            }
            return Err(status.as_error());
        }

        unsafe { (f.uregex_setTimeLimit)(re, 500, &mut status) };
        if status.is_failure() {
            unsafe { (f.uregex_close)(re) };
            return Err(status.as_error());
        }

        let mut status = icu_ffi::U_ZERO_ERROR;
        unsafe {
            (f.uregex_setText)(re, text.utf16.as_ptr(), text.utf16.len() as i32, &mut status)
        };
        if status.is_failure() {
            unsafe { (f.uregex_close)(re) };
            return Err(status.as_error());
        }

        Ok(Self { re, utf16_to_byte: text.utf16_to_byte.clone(), byte_len: text.utf8.len() })
    }

    /// # Safety
    /// `text` must contain the current complete buffer snapshot.
    pub unsafe fn set_text(&mut self, text: &Text, offset: usize) -> Result<()> {
        let f = assume_loaded();
        let mut status = icu_ffi::U_ZERO_ERROR;
        unsafe {
            (f.uregex_setText)(self.re, text.utf16.as_ptr(), text.utf16.len() as i32, &mut status)
        };
        if status.is_failure() {
            return Err(status.as_error());
        }

        self.utf16_to_byte = text.utf16_to_byte.clone();
        self.byte_len = text.utf8.len();
        self.reset(offset)
    }

    pub fn reset(&mut self, offset: usize) -> Result<()> {
        let f = assume_loaded();
        let utf16_offset = self.byte_to_utf16(offset);
        let mut status = icu_ffi::U_ZERO_ERROR;
        unsafe { (f.uregex_reset64)(self.re, utf16_offset as i64, &mut status) };
        if status.is_failure() {
            return Err(status.as_error());
        }
        Ok(())
    }

    pub fn find_next(&mut self) -> Result<Option<std::ops::Range<usize>>> {
        let f = assume_loaded();
        let mut status = icu_ffi::U_ZERO_ERROR;
        let found = unsafe { (f.uregex_findNext)(self.re, &mut status) };
        if status.is_failure() {
            return Err(status.as_error());
        }
        if !found {
            return Ok(None);
        }
        self.group_range(0)
    }

    pub fn group_count(&self) -> Result<i32> {
        let f = assume_loaded();
        let mut status = icu_ffi::U_ZERO_ERROR;
        let count = unsafe { (f.uregex_groupCount)(self.re, &mut status) };
        if status.is_failure() {
            return Err(status.as_error());
        }
        Ok(count)
    }

    pub fn group(&self, index: i32) -> Result<Option<std::ops::Range<usize>>> {
        self.group_range(index)
    }

    fn group_range(&self, index: i32) -> Result<Option<std::ops::Range<usize>>> {
        let f = assume_loaded();
        let mut status = icu_ffi::U_ZERO_ERROR;
        let start = unsafe { (f.uregex_start64)(self.re, index, &mut status) };
        if status.is_failure() {
            return Err(status.as_error());
        }
        if start < 0 {
            return Ok(None);
        }

        let mut status = icu_ffi::U_ZERO_ERROR;
        let end = unsafe { (f.uregex_end64)(self.re, index, &mut status) };
        if status.is_failure() {
            return Err(status.as_error());
        }
        if end < 0 {
            return Ok(None);
        }

        let Some(&byte_start) = self.utf16_to_byte.get(start as usize) else {
            return Ok(None);
        };
        let byte_end = self.utf16_to_byte.get(end as usize).copied().unwrap_or(self.byte_len);
        Ok(Some(byte_start..byte_end))
    }

    fn byte_to_utf16(&self, byte_offset: usize) -> usize {
        match self.utf16_to_byte.binary_search(&byte_offset) {
            Ok(offset) => offset,
            Err(offset) => offset.saturating_sub(1),
        }
    }
}

impl Drop for Regex {
    fn drop(&mut self) {
        if !self.re.is_null() {
            let f = assume_loaded();
            unsafe { (f.uregex_close)(self.re) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[ignore]
    #[test]
    fn init() {
        assert!(init_if_needed().is_ok());
    }

    #[test]
    fn test_compare_strings_ascii() {
        // Empty strings
        assert_eq!(compare_strings_ascii(b"", b""), Ordering::Equal);
        // Equal strings
        assert_eq!(compare_strings_ascii(b"hello", b"hello"), Ordering::Equal);
        // Different lengths
        assert_eq!(compare_strings_ascii(b"abc", b"abcd"), Ordering::Less);
        assert_eq!(compare_strings_ascii(b"abcd", b"abc"), Ordering::Greater);
        // Same chars, different cases - 1st char wins
        assert_eq!(compare_strings_ascii(b"AbC", b"aBc"), Ordering::Less);
        // Different chars, different cases
        assert_eq!(compare_strings_ascii(b"a", b"B"), Ordering::Less);
        assert_eq!(compare_strings_ascii(b"B", b"a"), Ordering::Greater);
        // Different chars, different cases - 2nd char wins, because it differs
        assert_eq!(compare_strings_ascii(b"hallo", b"Hello"), Ordering::Less);
        assert_eq!(compare_strings_ascii(b"Hello", b"hallo"), Ordering::Greater);
    }

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
                    assert_eq!(folded, "strasse");
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    }
}
