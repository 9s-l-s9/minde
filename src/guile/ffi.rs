//! Minimal hand-written bindings to libguile (no bindgen).
//!
//! `SCM` values are opaque and word-sized both when Guile is built with the
//! "strict typing" union representation and when it is a plain `scm_t_bits`
//! integer, so representing it as a transparent pointer newtype is ABI-safe
//! as long as we never inspect its bits from Rust and only ever pass it
//! back into libguile.

use std::os::raw::{c_char, c_int, c_void};

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Scm(pub *mut c_void);

unsafe impl Send for Scm {}

// `scm_from_bool`, `SCM_BOOL_T`/`SCM_BOOL_F` are inline/macro-only in the
// Guile headers, so we hardcode the immediate flag encodings from scm.h:
// SCM_MAKIFLAG_BITS(n) = (n << 8) + scm_tc8_flag(4). Stable across Guile 3.x.
pub const SCM_BOOL_F: Scm = Scm(0x004 as *mut c_void);
pub const SCM_BOOL_T: Scm = Scm(0x404 as *mut c_void);
/// Passed by Guile for a missing optional gsubr argument (iflag 9, see
/// libguile/tags.h; same encoding scheme as the booleans above).
pub const SCM_UNDEFINED: Scm = Scm(0x904 as *mut c_void);
/// The empty list `'()` (SCM_EOL: iflag 3, same encoding scheme).
pub const SCM_EOL: Scm = Scm(0x304 as *mut c_void);

pub fn scm_from_bool_inline(x: bool) -> Scm {
    if x { SCM_BOOL_T } else { SCM_BOOL_F }
}

/// Body callback for `scm_internal_catch`.
pub type CatchBody = unsafe extern "C" fn(data: *mut c_void) -> Scm;
/// Handler callback for `scm_internal_catch`: `(data, key, args) -> SCM`.
pub type CatchHandler = unsafe extern "C" fn(data: *mut c_void, key: Scm, args: Scm) -> Scm;
/// Signature used when registering subrs with `scm_c_define_gsubr`.
/// The real arity is encoded separately; we cast concrete `extern "C" fn(..)
/// -> Scm` pointers to this type when registering.
pub type Gsubr = unsafe extern "C" fn() -> Scm;

unsafe extern "C" {
    pub fn scm_init_guile();

    pub fn scm_c_primitive_load(filename: *const c_char) -> Scm;
    pub fn scm_c_eval_string(expr: *const c_char) -> Scm;

    pub fn scm_c_define_gsubr(
        name: *const c_char,
        req: c_int,
        opt: c_int,
        rst: c_int,
        fcn: Gsubr,
    ) -> Scm;

    pub fn scm_call_0(proc: Scm) -> Scm;
    pub fn scm_call_1(proc: Scm, arg1: Scm) -> Scm;
    pub fn scm_call_2(proc: Scm, arg1: Scm, arg2: Scm) -> Scm;
    pub fn scm_call_3(proc: Scm, arg1: Scm, arg2: Scm, arg3: Scm) -> Scm;
    pub fn scm_call_4(proc: Scm, arg1: Scm, arg2: Scm, arg3: Scm, arg4: Scm) -> Scm;
    pub fn scm_call_5(proc: Scm, arg1: Scm, arg2: Scm, arg3: Scm, arg4: Scm, arg5: Scm) -> Scm;

    pub fn scm_from_utf8_string(s: *const c_char) -> Scm;
    /// Returns a malloc'd, NUL-terminated UTF-8 buffer; caller must free() it.
    pub fn scm_to_utf8_stringn(str_: Scm, lenp: *mut usize) -> *mut c_char;

    pub fn scm_to_bool(x: Scm) -> c_int;

    pub fn scm_from_int64(x: i64) -> Scm;
    pub fn scm_to_int64(x: Scm) -> i64;
    pub fn scm_integer_p(x: Scm) -> Scm;
    pub fn scm_string_p(x: Scm) -> Scm;
    pub fn scm_symbol_p(x: Scm) -> Scm;
    pub fn scm_symbol_to_string(x: Scm) -> Scm;
    pub fn scm_from_utf8_symbol(s: *const c_char) -> Scm;
    pub fn scm_pair_p(x: Scm) -> Scm;
    pub fn scm_null_p(x: Scm) -> Scm;
    pub fn scm_car(x: Scm) -> Scm;
    pub fn scm_cdr(x: Scm) -> Scm;

    pub fn scm_list_4(a: Scm, b: Scm, c: Scm, d: Scm) -> Scm;
    /// `(cons a b)` -- for building lists of arbitrary length from Rust.
    pub fn scm_cons(a: Scm, b: Scm) -> Scm;

    /// Looks up a top-level variable object by name (throws if unbound).
    pub fn scm_c_lookup(name: *const c_char) -> Scm;
    /// Dereferences a variable object obtained from `scm_c_lookup`.
    pub fn scm_variable_ref(var: Scm) -> Scm;

    pub fn scm_internal_catch(
        tag: Scm,
        body: CatchBody,
        body_data: *mut c_void,
        handler: CatchHandler,
        handler_data: *mut c_void,
    ) -> Scm;
}

unsafe extern "C" {
    pub fn free(p: *mut c_void);
}
