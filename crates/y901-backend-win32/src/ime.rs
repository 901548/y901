//! 极简 Win32 IME 绑定。
//!
//! 只做终端需要的事：拿组合串、拿提交串、把候选窗钉在光标处。
//! `windows-sys` 不覆盖 imm32，这里手写函数声明。

use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};

pub const WM_IME_STARTCOMPOSITION: u32 = 0x010D;
pub const WM_IME_ENDCOMPOSITION: u32 = 0x010E;
pub const WM_IME_COMPOSITION: u32 = 0x010F;

pub const GCS_COMPSTR: u32 = 0x0008;
pub const GCS_RESULTSTR: u32 = 0x0800;
#[allow(dead_code)] // 留作后续接线：组合串内的光标位置
pub const GCS_CURSORPOS: u32 = 0x0080;

const CFS_POINT: u32 = 0x0002;

#[link(name = "imm32")]
unsafe extern "system" {
    fn ImmGetContext(hwnd: HWND) -> *mut c_void;
    fn ImmReleaseContext(hwnd: HWND, himc: *mut c_void) -> i32;
    fn ImmGetCompositionStringW(himc: *mut c_void, index: u32, buf: *mut c_void, len: u32) -> i32;
    fn ImmSetCompositionWindow(himc: *mut c_void, form: *const CompositionForm) -> i32;
    fn ImmSetCandidateWindow(himc: *mut c_void, form: *const CandidateForm) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CompositionForm {
    style: u32,
    current_position: POINT,
    area: [i32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CandidateForm {
    index: u32,
    style: u32,
    current_position: POINT,
    area: [i32; 4],
}

/// 组合态：正在拼写的拼音（绝不进 Grid）。
#[derive(Default)]
pub struct ImeState {
    /// UTF-16 组合串。
    pub composing: Vec<u16>,
    /// 是否处于组合中。
    pub active: bool,
    /// 光标位置（客户区像素），用于画覆盖层。
    /// 留作后续接线：候选窗定位需要它，当前由组件自绘。
    #[allow(dead_code)]
    pub caret: (f32, f32),
}

impl ImeState {
    pub fn composing_text(&self) -> String {
        String::from_utf16_lossy(&self.composing)
    }

    pub fn clear(&mut self) {
        self.composing.clear();
        self.active = false;
    }
}

/// 从 lParam 取出本次 COMPOSITION 里可用的位掩码。
pub fn composition_flags(lparam: LPARAM) -> u32 {
    lparam as u32
}

/// 读 GCS_COMPSTR；空串时返回 None。
pub unsafe fn get_composition_string(hwnd: HWND, flag: u32) -> Option<Vec<u16>> {
    let himc = ImmGetContext(hwnd);
    if himc.is_null() {
        return None;
    }
    let bytes = ImmGetCompositionStringW(himc, flag, null_mut(), 0);
    if bytes <= 0 {
        ImmReleaseContext(hwnd, himc);
        return None;
    }
    let units = bytes as usize / 2;
    let mut buf = vec![0u16; units];
    let written =
        ImmGetCompositionStringW(himc, flag, buf.as_mut_ptr() as *mut c_void, bytes as u32);
    ImmReleaseContext(hwnd, himc);
    if written <= 0 {
        return None;
    }
    buf.truncate(written as usize / 2);
    Some(buf)
}

/// 把候选窗与组合窗钉在客户区 (x, y)。
/// 把候选窗/组合窗钉在客户区 (x, y)。
///
/// 留作后续接线：TextInput 需要它把 IME 候选框贴到光标处，
/// 当前应用还没调用（终端组件自己画预编辑串）。
#[allow(dead_code)]
pub unsafe fn set_composition_position(hwnd: HWND, x: f32, y: f32) {
    let himc = ImmGetContext(hwnd);
    if himc.is_null() {
        return;
    }
    let pos = POINT {
        x: x as i32,
        y: y as i32,
    };
    let composition = CompositionForm {
        style: CFS_POINT,
        current_position: pos,
        area: [0; 4],
    };
    ImmSetCompositionWindow(himc, &composition);
    let candidate = CandidateForm {
        index: 0,
        style: CFS_POINT,
        current_position: pos,
        area: [0; 4],
    };
    ImmSetCandidateWindow(himc, &candidate);
    ImmReleaseContext(hwnd, himc);
}

/// 消息处理结果：由主循环决定副作用（写 PTY / 重绘）。
#[derive(Debug, PartialEq)]
pub enum ImeAction {
    /// 没处理，交给 DefWindowProc。
    Unhandled,
    /// 已处理，只需重绘（组合串变了）。
    Redraw,
    /// 已处理，需要把这段文字写进 PTY。
    Commit(String),
}

/// 消息路由。
pub unsafe fn handle_ime_message(
    hwnd: HWND,
    msg: u32,
    lparam: LPARAM,
    state: &mut ImeState,
) -> ImeAction {
    match msg {
        WM_IME_STARTCOMPOSITION => {
            state.active = true;
            state.composing.clear();
            ImeAction::Redraw
        }
        WM_IME_COMPOSITION => {
            let flags = composition_flags(lparam);

            // 先处理提交串：选定的汉字交给主循环写 PTY。
            if flags & GCS_RESULTSTR != 0 {
                let committed = get_composition_string(hwnd, GCS_RESULTSTR)
                    .map(|text| String::from_utf16_lossy(&text))
                    .filter(|s| !s.is_empty());
                state.composing.clear();
                state.active = false;
                return match committed {
                    Some(text) => ImeAction::Commit(text),
                    None => ImeAction::Redraw,
                };
            }

            // 再更新组合串：只进覆盖层，绝不进 Grid。
            if flags & GCS_COMPSTR != 0 {
                match get_composition_string(hwnd, GCS_COMPSTR) {
                    Some(text) => {
                        state.composing = text;
                        state.active = !state.composing.is_empty();
                    }
                    None => state.clear(),
                }
                return ImeAction::Redraw;
            }
            ImeAction::Redraw
        }
        WM_IME_ENDCOMPOSITION => {
            state.clear();
            ImeAction::Redraw
        }
        _ => ImeAction::Unhandled,
    }
}

/// 让编译器闭嘴：`WPARAM` / `LRESULT` 在签名里保留以便未来扩展。
#[allow(dead_code)]
fn _unused(_: WPARAM, _: LRESULT) {}
