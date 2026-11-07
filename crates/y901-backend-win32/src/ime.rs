//! 极简 Win32 IME 绑定。
//!
//! 只做终端需要的事：拿组合串、拿提交串、把候选窗钉在光标处。
//! `windows-sys` 不覆盖 imm32，这里手写函数声明。

use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};

/// 开始组字（输入法开始拼一个候选串）。
pub const WM_IME_STARTCOMPOSITION: u32 = 0x010D;
/// 结束组字。
pub const WM_IME_ENDCOMPOSITION: u32 = 0x010E;
/// 组字更新；`lParam` 的 `GCS_*` 位标出本次携带了什么。
pub const WM_IME_COMPOSITION: u32 = 0x010F;

/// 本次携带的是"正在组字的中间串"（预编辑文本）。
pub const GCS_COMPSTR: u32 = 0x0008;
/// 本次携带的是"已确定的最终串"，应当作为 `UiEvent::Text` 提交。
pub const GCS_RESULTSTR: u32 = 0x0800;

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
    // 诊断 API：预编辑光标位置，供覆盖层绘制
    // 归类：诊断 API —— `caret` 只被 `Y901_IME_DUMP` 读，供 IME 定位排查
    #[allow(dead_code)]
    pub caret: (f32, f32),
}

impl ImeState {
    /// 当前预编辑串。为什么与"已提交文本"分开：预编辑只画在覆盖层，绝不进网格——否则撤销一次组字就要回滚整屏。
    pub fn composing_text(&self) -> String {
        String::from_utf16_lossy(&self.composing)
    }

    /// 清空预编辑状态（组字被取消、或焦点离开输入框时调用）。
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
///
/// # Safety
///
/// 调用者必须保证：
/// - `hwnd` 是有效且未销毁的窗口句柄，并且**调用线程**是该窗口的创建线程
///   （IMM 上下文按线程绑定，跨线程调用 `ImmGetContext` 会拿到无效上下文）；
/// - `flag` 是 IMM 认识的 GCS_* 位（如 `GCS_COMPSTR` / `GCS_RESULTSTR`）。
///
/// 违反会导致：`ImmGetContext` 返回空指针（本函数已判空并返回 `None`，安全），
/// 或在错误的线程上读取到别的输入法上下文，返回垃圾 UTF-16 数据。
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

/// 把候选窗/组合窗钉在客户区 (x, y)。
///
/// 留作后续接线：TextInput 需要它把 IME 候选框贴到光标处，
/// 当前应用还没调用（终端组件自己画预编辑串）。
///
/// # Safety
///
/// 调用者必须保证：
/// - `hwnd` 有效且未销毁，且调用发生在该窗口的创建线程上（IMM 上下文按线程绑定）；
/// - `x`、`y` 是**客户区**像素坐标（非屏幕坐标），并且落在合理范围内
///   （超出 `i32` 表示范围会被截断）。
///
/// 违反会导致：`ImmGetContext` 返回空指针（本函数已判空返回，安全），
/// 或把候选窗钉到错误位置；不会造成内存不安全。
// 诊断 API：`set_composition_position()`：输入法候选窗跟随，当前未接线
// 归类：诊断 API —— `set_composition_position` 由 window.rs 的 IME 路径调用，测试不覆盖
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

/// 消息路由：把 WM_IME_* 翻译成 [`ImeAction`]。
///
/// # Safety
///
/// 调用者必须保证：
/// - `hwnd` 有效且未销毁，且调用发生在该窗口的创建线程上；
/// - `msg` 必须是 `WM_IME_STARTCOMPOSITION` / `WM_IME_COMPOSITION` /
///   `WM_IME_ENDCOMPOSITION` 之一 —— `lparam` 的解释随 `msg` 变化，
///   对普通消息调用会把 `lparam` 当位掩码解析；
/// - `lparam` 必须来自对应 `msg` 的窗口过程调用（不可伪造）。
///
/// 违反会导致：把任意整数当 GCS 位掩码，进而调用 `ImmGetCompositionStringW`
/// 读取不存在的组合串（返回错误码，本函数会降级为 `Redraw`），
/// 或对错误窗口取上下文导致候选窗错位；不会造成内存不安全。
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
// 归类：编译见证 —— `_unused` 用来证明若干类型被使用，不是运行路径
#[allow(dead_code)]
fn _unused(_: WPARAM, _: LRESULT) {}
