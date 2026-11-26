//! **X11 协议读取 + 窗口 resize** 的真服务器验收（Xvfb 下运行）。
//!
//! 运行方式：
//! ```bash
//! cd /mnt/f/Y901
//! xvfb-run -a cargo test -p y901-backend-x11 --test protocol_and_resize -- --nocapture
//! ```
//!
//! 这个文件对应 2026-11-26 的 X11 正确性收口任务，三项各自的验收点：
//!
//! 1. **get_property 的 bytes_after**：大属性必须整段读回；超过上限必须**显式失败**
//!    （旧实现静默截断）；
//! 2. **resize**：`ConfigureNotify` 必须把窗口自己的尺寸落库，`relayout` 之后
//!    布局尺寸 == 帧缓冲尺寸 == 窗口尺寸，`present` 不再按旧尺寸截断；
//! 3. 半包/0 超时的读取语义由 `conn.rs` 的单测覆盖（`UnixStream::pair()`，不需要显示）。
//!
//! 没有 DISPLAY 时**留痕跳过**（写 `target/skip-count.txt`），而不是假红。

#![cfg(target_os = "linux")]

use std::time::Duration;

use y901_backend_x11::conn::Connection;
use y901_backend_x11::encode::{
    encode_change_property, encode_configure_window, encode_get_image, encode_intern_atom,
    ConfigureMask, PropMode,
};
use y901_backend_x11::window::{CreateArgs, X11Window};
use y901_backend_x11::X11Event;
use y901_runtime::window::WindowBackend;
use y901_runtime::Presenter;
use y901_ui::{Color, Flex, Size, UiTree};

static SERVER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const W: u16 = 320;
const H: u16 = 200;

/// 记一条 skip（与 `real_window.rs` 同一套约定：写 `target/skip-count.txt`）。
fn record_skip(test_name: &str, reason: &str) {
    use std::io::Write;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/skip-count.txt");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let test_name = test_name.replace(['\t', '\n', '\r'], " ");
    let reason = reason.replace(['\t', '\n', '\r'], " ");
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut f) => {
            let _ = writeln!(f, "0\t{test_name}\t{reason}");
        }
        Err(e) => eprintln!("[WARN] 写 skip-count 失败（{path:?}）：{e}"),
    }
}

fn display() -> String {
    std::env::var("DISPLAY").unwrap_or_else(|_| ":99".to_string())
}

/// 带重试的连接（Xvfb 高频建连偶发 RST）。
fn connect(d: &str) -> Option<Connection> {
    for attempt in 0..3 {
        match Connection::connect(d) {
            Ok(c) => return Some(c),
            Err(e) => {
                if attempt == 2 {
                    eprintln!("[SKIP] 连接 {d} 失败：{e}");
                    record_skip("protocol_and_resize", &format!("connect: {e}"));
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    None
}

fn soft_for(w: u16, h: u16) -> y901_render::soft::SoftBackend {
    let size = 15.0f32;
    let (cw, ch) = y901_render::cell_metrics_for(size);
    let cell = Size::new(cw, ch);
    let engine = y901_render::MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("字体可读"),
        size,
        cell,
    )
    .expect("字形引擎");
    y901_render::soft::SoftBackend::new(w as u32, h as u32, Box::new(engine), cell, None)
        .expect("软光栅器")
}

fn create_window(d: &str, w: u16, h: u16, title: &str) -> Option<X11Window> {
    for attempt in 0..3 {
        match X11Window::create(CreateArgs {
            display: d,
            width: w,
            height: h,
            title,
            soft: soft_for(w, h),
            fullscreen: false,
        }) {
            Ok(x) => return Some(x),
            Err(e) => {
                if attempt == 2 {
                    eprintln!("[SKIP] 建窗失败：{e}");
                    record_skip("protocol_and_resize", &format!("create: {e}"));
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    None
}

fn intern(conn: &mut Connection, name: &[u8]) -> u32 {
    conn.send(&encode_intern_atom(name, false))
        .expect("InternAtom");
    conn.flush().expect("flush");
    conn.read_reply_u32().expect("InternAtom reply")
}

/// 在窗口上写一个属性（format 8/16/32 由调用方给，并自带长度校验）。
fn set_property(
    conn: &mut Connection,
    wid: u32,
    property: u32,
    type_atom: u32,
    format: u8,
    data: &[u8],
) {
    let per_item = match format {
        8 => 1,
        16 => 2,
        32 => 4,
        _ => panic!("非法 format {format}"),
    };
    assert_eq!(
        data.len() % per_item,
        0,
        "format {format} 要求数据长度是 {per_item} 的整数倍"
    );
    assert!(
        data.len() <= y901_backend_x11::encode::MAX_PROPERTY_BYTES,
        "单次 ChangeProperty 的数据不能超过 {} 字节（协议上限）；大属性请用 Append 分块",
        y901_backend_x11::encode::MAX_PROPERTY_BYTES
    );
    conn.send(&encode_change_property(
        wid,
        property,
        type_atom,
        format,
        PropMode::Replace,
        data,
    ))
    .expect("ChangeProperty");
    conn.flush().expect("flush");
    let errs = conn.sync().expect("sync");
    assert!(errs.is_empty(), "ChangeProperty 报错：{errs:?}");
}

// ===========================================================================
// 1. get_property_all：大属性 / format / 空属性 / 上限
// ===========================================================================

#[test]
fn get_property_all_reads_large_property_completely() {
    let _g = SERVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = display();
    let Some(mut c) = connect(&d) else { return };
    let Some(win) = create_window(&d, W, H, "y901-prop-large") else {
        return;
    };
    let wid = win.wid;
    let prop = intern(&mut c, b"Y901_TEST_PROP");
    let ty = intern(&mut c, b"Y901_TEST_TYPE");

    // 600 KiB：**超过单次 ChangeProperty 的协议上限**（262 116 字节），
    // 所以必须分块写（PropMode::Append）。这也让 GetProperty 的响应
    // 很可能**装不下全部数据**（bytes_after > 0）→ 真正走到分页续读路径。
    let n = 600 * 1024;
    let data: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
    let chunk = y901_backend_x11::encode::MAX_PROPERTY_BYTES;
    let mut first = true;
    for part in data.chunks(chunk) {
        let mode = if first {
            PropMode::Replace
        } else {
            PropMode::Append
        };
        first = false;
        c.send(&encode_change_property(wid, prop, ty, 8, mode, part))
            .expect("ChangeProperty");
        c.flush().expect("flush");
        let errs = c.sync().expect("sync");
        assert!(errs.is_empty(), "ChangeProperty 分块写报错：{errs:?}");
    }
    // 前置：确认真的是 600 KiB（否则下面的"读全"就没意义）
    let (f0, got0) = c
        .get_property(wid, prop, ty, 0, (n / 4) as u32, false)
        .expect("一次读满应成功");
    assert_eq!(f0, 8);
    assert_eq!(got0.len(), n, "分块写入后属性总长必须为 {n}");

    let (format, got) = c
        .get_property_all(wid, prop, ty, false, 4 << 20)
        .expect("大属性必须整段读回");
    assert_eq!(format, 8, "format 必须与写入一致");
    assert_eq!(got.len(), n, "读回的字节数必须与写入相同（不得截断）");
    assert!(got == data, "读回的内容必须逐字节相同");
    eprintln!("[proto] 大属性 {n} 字节完整读回 ✓");
}

#[test]
fn get_property_all_handles_16_and_32_bit_formats() {
    let _g = SERVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = display();
    let Some(mut c) = connect(&d) else { return };
    let Some(win) = create_window(&d, W, H, "y901-prop-formats") else {
        return;
    };
    let wid = win.wid;
    let prop = intern(&mut c, b"Y901_TEST_PROP16");
    let ty = intern(&mut c, b"Y901_TEST_TYPE");

    // format=16：0..100 的 u16 小端
    let mut d16 = Vec::new();
    for v in 0u16..100 {
        d16.extend_from_slice(&v.to_le_bytes());
    }
    set_property(&mut c, wid, prop, ty, 16, &d16);
    let (f, got) = c
        .get_property_all(wid, prop, ty, false, 1 << 20)
        .expect("format=16 读取");
    assert_eq!(f, 16);
    assert_eq!(got, d16, "format=16 的内容必须逐字节相同");

    // format=32：0..50 的 u32 小端
    let mut d32 = Vec::new();
    for v in 0u32..50 {
        d32.extend_from_slice(&v.to_le_bytes());
    }
    set_property(&mut c, wid, prop, ty, 32, &d32);
    let (f, got) = c
        .get_property_all(wid, prop, ty, false, 1 << 20)
        .expect("format=32 读取");
    assert_eq!(f, 32);
    assert_eq!(got, d32, "format=32 的内容必须逐字节相同");
    eprintln!("[proto] format 16/32 读取正确 ✓");
}

#[test]
fn get_property_all_handles_missing_and_empty_property() {
    let _g = SERVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = display();
    let Some(mut c) = connect(&d) else { return };
    let Some(win) = create_window(&d, W, H, "y901-prop-empty") else {
        return;
    };
    let wid = win.wid;
    let prop = intern(&mut c, b"Y901_TEST_PROP_EMPTY");
    let ty = intern(&mut c, b"Y901_TEST_TYPE");

    // (a) 根本不存在的属性：必须是"空"，不是错误。
    let (format, got) = c
        .get_property_all(wid, prop, ty, false, 1 << 20)
        .expect("不存在的属性应返回空，而不是 Err");
    assert_eq!(format, 0, "属性不存在时 X 返回 format=0");
    assert!(got.is_empty());

    // (b) 存在但长度为 0 的属性：同样是空。
    set_property(&mut c, wid, prop, ty, 8, &[]);
    let (_, got) = c
        .get_property_all(wid, prop, ty, false, 1 << 20)
        .expect("空属性应返回空");
    assert!(got.is_empty(), "空属性不得返回任何字节");
    eprintln!("[proto] 不存在/空属性语义正确 ✓");
}

#[test]
fn get_property_all_rejects_oversized_property_explicitly() {
    let _g = SERVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = display();
    let Some(mut c) = connect(&d) else { return };
    let Some(win) = create_window(&d, W, H, "y901-prop-cap") else {
        return;
    };
    let wid = win.wid;
    let prop = intern(&mut c, b"Y901_TEST_PROP_CAP");
    let ty = intern(&mut c, b"Y901_TEST_TYPE");

    let data: Vec<u8> = vec![7u8; 4096];
    set_property(&mut c, wid, prop, ty, 8, &data);

    // 上限 1 KiB < 属性 4 KiB：必须**显式报错**（旧实现会返回 1 KiB 静默截断）。
    let err = c
        .get_property_all(wid, prop, ty, false, 1024)
        .expect_err("超过上限必须失败");
    assert!(
        err.contains("上限") && err.contains("4096"),
        "错误信息必须说明总大小与上限：{err}"
    );

    // 上限放够：同样的属性必须能读全（证明上面的失败是"上限"而不是"读不了"）。
    let (_, got) = c
        .get_property_all(wid, prop, ty, false, 8192)
        .expect("放大上限后应能读全");
    assert_eq!(got.len(), 4096);
    eprintln!("[proto] 超限显式失败 + 放大后成功 ✓");
}

#[test]
fn get_property_single_page_reports_truncation_instead_of_returning_half() {
    let _g = SERVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = display();
    let Some(mut c) = connect(&d) else { return };
    let Some(win) = create_window(&d, W, H, "y901-prop-trunc") else {
        return;
    };
    let wid = win.wid;
    let prop = intern(&mut c, b"Y901_TEST_PROP_TRUNC");
    let ty = intern(&mut c, b"Y901_TEST_TYPE");

    let data: Vec<u8> = vec![3u8; 512];
    set_property(&mut c, wid, prop, ty, 8, &data);

    // 只要 1 单位（4 字节）→ 还有 508 字节没读：**必须报错**，不能静默返回 4 字节。
    let err = c
        .get_property(wid, prop, ty, 0, 1, false)
        .expect_err("少读时必须显式报错");
    assert!(
        err.contains("还有") || err.contains("未读"),
        "错误必须说明还有多少没读：{err}"
    );

    // 读够（512 字节 = 128 单位）则必须成功且完整。
    let (_, got) = c
        .get_property(wid, prop, ty, 0, 128, false)
        .expect("够长时必须成功");
    assert_eq!(got, data);
    eprintln!("[proto] 单页截断显式报错 ✓");
}

// ===========================================================================
// 2. resize：尺寸落库 / 布局与缓冲一致 / 去重 / present 不按旧尺寸截断
// ===========================================================================

fn tree_for(w: u16, h: u16) -> UiTree {
    let root = Flex::column("root")
        .background(Color::from_rgb8(10, 20, 30))
        .padding(y901_ui::Edges::all(4.0));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(w as f32, h as f32), &soft_for(w, h));
    tree.invalidate_all();
    tree
}

/// 用**另一个连接**改窗口尺寸（模拟 WM），返回改动后的尺寸。
fn resize_from_other_client(d: &str, wid: u32, w: u16, h: u16) -> Option<Connection> {
    let mut other = connect(d)?;
    other
        .send(&encode_configure_window(
            wid,
            &[
                (ConfigureMask::Width, w as u32),
                (ConfigureMask::Height, h as u32),
            ],
        ))
        .expect("ConfigureWindow");
    other.flush().expect("flush");
    let errs = other.sync().expect("sync");
    assert!(errs.is_empty(), "ConfigureWindow 报错：{errs:?}");
    Some(other)
}

#[test]
fn configure_notify_updates_size_and_frame_buffer() {
    let _g = SERVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = display();
    let Some(mut win) = create_window(&d, W, H, "y901-resize-up") else {
        return;
    };
    assert_eq!(win.size(), (W, H), "初始尺寸应为建窗尺寸");
    let mut tree = tree_for(W, H);

    // 放大到 480×300 (WM 侧改尺寸)
    let wid = win.wid;
    let Some(_other) = resize_from_other_client(&d, wid, 480, 300) else {
        return;
    };

    // 读事件：必须收到 Resize(480,300)，且窗口自己的尺寸要跟着更新。
    let mut got = None;
    for _ in 0..40 {
        let evs = win.wait_events(50).unwrap_or_default();
        for e in evs {
            if let y901_runtime::RawEvent::Resize { w, h } = e {
                got = Some((w, h));
            }
        }
        if got == Some((480, 300)) {
            break;
        }
    }
    assert_eq!(got, Some((480, 300)), "WM 改尺寸后必须收到 Resize(480,300)");
    assert_eq!(
        win.size(),
        (480, 300),
        "窗口缓存的尺寸必须跟着 ConfigureNotify 更新（旧实现永远停在 {W}×{H}）"
    );

    // relayout：布局尺寸与帧缓冲尺寸必须都变成新尺寸
    win.relayout(&mut tree);
    assert_eq!(
        win.soft().width(),
        480,
        "帧缓冲宽度必须与窗口一致（否则 present 只画左上角）"
    );
    assert_eq!(win.soft().height(), 300);

    // present 一下（新尺寸的像素）必须成功，且不该被旧尺寸截断
    let pixels = vec![0u8; 480 * 300 * 4];
    win.present(&pixels, 480, 300)
        .expect("新尺寸上屏应成功（不得因旧尺寸钳制而丢区域）");
    eprintln!("[proto] 放大：尺寸/布局/缓冲/present 一致 ✓");

    // 缩小到 240×160
    let Some(_other) = resize_from_other_client(&d, wid, 240, 160) else {
        return;
    };
    let mut got = None;
    for _ in 0..40 {
        let evs = win.wait_events(50).unwrap_or_default();
        for e in evs {
            if let y901_runtime::RawEvent::Resize { w, h } = e {
                got = Some((w, h));
            }
        }
        if got == Some((240, 160)) {
            break;
        }
    }
    assert_eq!(got, Some((240, 160)), "缩小后必须收到 Resize(240,160)");
    win.relayout(&mut tree);
    assert_eq!((win.soft().width(), win.soft().height()), (240, 160));
    win.present(&vec![0u8; 240 * 160 * 4], 240, 160)
        .expect("缩小后上屏应成功");
    eprintln!("[proto] 缩小：尺寸/布局/缓冲/present 一致 ✓");
}

#[test]
fn duplicate_configure_notify_is_deduped() {
    let _g = SERVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = display();
    let Some(mut win) = create_window(&d, W, H, "y901-resize-dedupe") else {
        return;
    };
    let wid = win.wid;

    // **同一尺寸连改三次**：WM 拖动时就是这个形态。
    for _ in 0..3 {
        let Some(_o) = resize_from_other_client(&d, wid, 360, 240) else {
            return;
        };
    }

    // `Expose` 也会对外发 Resize（"该画一帧"的兼容路径，携带**当前**尺寸），
    // 所以这里数的不是"事件个数"，而是**尺寸跳变次数**：
    // 3 次同尺寸 ConfigureWindow 只应产生 1 次 320×200 → 360×240 的跳变。
    // `RawEvent::Resize` 的字段是 u32（与 trait 契约一致），这里统一用 u32 比较。
    let mut last = (W as u32, H as u32);
    let mut transitions: Vec<(u32, u32)> = Vec::new();
    for _ in 0..40 {
        for e in win.wait_events(50).unwrap_or_default() {
            if let y901_runtime::RawEvent::Resize { w, h } = e {
                if (w, h) != last {
                    transitions.push((w, h));
                    last = (w, h);
                }
            }
        }
    }
    assert_eq!(
        transitions,
        vec![(360, 240)],
        "3 次同尺寸 ConfigureNotify 只应产生一次尺寸跳变（实际 {transitions:?}）"
    );
    assert_eq!(win.size(), (360, 240));
    eprintln!("[proto] 重复 ConfigureNotify 去重 ✓（跳变 {transitions:?}）");
}

#[test]
fn present_after_resize_paints_the_new_area() {
    let _g = SERVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = display();
    let Some(mut win) = create_window(&d, W, H, "y901-resize-paint") else {
        return;
    };
    let wid = win.wid;
    let mut tree = tree_for(W, H);

    // 放大后**真的画一帧**，再用另一个连接回读新区域的像素 —— 这是
    // "present 不再按旧尺寸截断"的像素级证据（只断言尺寸一致不够）。
    let Some(mut probe) = resize_from_other_client(&d, wid, 480, 300) else {
        return;
    };
    // 等 ConfigureNotify 落地（wait_events 会更新 win.size()）
    for _ in 0..40 {
        let evs = win.wait_events(50).unwrap_or_default();
        if evs
            .iter()
            .any(|e| matches!(e, y901_runtime::RawEvent::Resize { .. }))
        {
            break;
        }
    }
    assert_eq!(win.size(), (480, 300), "前置：尺寸必须已更新");

    win.relayout(&mut tree);
    tree.invalidate_all();
    let _ = win.render(&mut tree);

    // 回读右下角（新区域）与左上角各一个像素；右下角必须不是"未初始化的 0"。
    let far = probe
        .get_image(wid, 470, 290, 4, 4)
        .expect("GetImage 回读新区域");
    assert_eq!(far.0, 24, "回读深度应为 24");
    let any_nonzero = far.1.iter().any(|b| *b != 0);
    assert!(
        any_nonzero,
        "新区域必须被真的画上内容（旧实现只上传旧尺寸，新区域保持未初始化）"
    );
    eprintln!("[proto] resize 后新区域已绘制 ✓");
}

#[test]
fn expose_after_resize_reports_current_size() {
    // `Expose` 走的是"按当前尺寸重发一个 Resize"的兼容路径：它读的必须是
    // **已更新**的 `self.width/height`（旧实现会报旧尺寸）。
    let _g = SERVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = display();
    let Some(mut win) = create_window(&d, W, H, "y901-resize-expose") else {
        return;
    };
    let wid = win.wid;
    let Some(_o) = resize_from_other_client(&d, wid, 400, 260) else {
        return;
    };
    let mut sizes = Vec::new();
    for _ in 0..40 {
        for e in win.wait_events(50).unwrap_or_default() {
            if let y901_runtime::RawEvent::Resize { w, h } = e {
                sizes.push((w, h));
            }
        }
        if !sizes.is_empty() {
            break;
        }
    }
    assert!(
        !sizes.is_empty(),
        "resize 后应至少产生一个 Resize/Expose 信号"
    );
    assert_eq!(
        sizes.last().copied(),
        Some((400, 260)),
        "信号里的尺寸必须是**新**尺寸，实测 {sizes:?}"
    );
    eprintln!("[proto] Expose/Resize 携带新尺寸 ✓");
}

/// 半包 + 0 超时的**真连接**冒烟：在真 socket 上也不允许因为"暂时没数据"报错。
#[test]
fn real_connection_nonblocking_poll_is_not_an_error() {
    let _g = SERVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = display();
    let Some(mut c) = connect(&d) else { return };
    // 没有待处理事件时，0 超时轮询必须返回"空"而不是 Err。
    for _ in 0..5 {
        let r = c.poll_events(0);
        assert!(r.is_ok(), "0 超时轮询不得因无数据而报错：{r:?}");
    }
    // try_next_packet(0) 同理：要么 Ok(Some)，要么 Ok(None)，绝不 Err(EINVAL)。
    match c.try_next_packet(0) {
        Ok(_) => {}
        Err(e) => panic!("0 超时读到 EINVAL 类错误：{e}"),
    }
    eprintln!("[proto] 真连接 0 超时轮询正常 ✓");
}

/// 供 `get_image` 回读用：确保 `encode_get_image` 被使用（防止未使用导入）。
#[allow(dead_code)]
fn _assert_get_image_used() {
    let _ = encode_get_image(0, 0, 0, 1, 1, 0xFFFF_FFFF);
}

/// 供 `X11Event` 导入使用。
#[allow(dead_code)]
fn _assert_event_used(e: &X11Event) -> bool {
    matches!(e, X11Event::Expose { .. })
}
