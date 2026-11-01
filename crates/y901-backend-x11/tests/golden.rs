//! X11 协议编码层的 **golden test**。
//!
//! 每个期望字节都是**照着规范的字段布局手算出来的**，
//! 并在注释里标上字段名与偏移——这样一旦实现与规范不符，读者可以直接定位。
//!
//! 诚实声明：golden test 验证的是"实现与我理解的规范一致"。
//! 最终真相只能由 **S2 第二步的真服务器握手**畑开（服务器会拒绝格式错的包）。

use y901_backend_x11::encode::{CreateWindowArgs, ImageFormat, PropMode, PutImageArgs, ValueMask, WindowClass};
use y901_backend_x11::{
    decode_event, encode_change_property, encode_create_window, encode_get_input_focus,
    encode_intern_atom, encode_map_window, encode_no_operation, encode_put_image,
    encode_setup_request, pad4, DecodeError, X11Event,
};

// ---------------------------------------------------------------- 基础工具

/// 验证：`got` 与期望字节逐位相同，不同时报出**第一个不同的位置**。
///
/// 为什么不直接 `assert_eq!(got, want)`：一个 100 字节的数组打印出来根本看不出哪里错。
fn assert_bytes(got: &[u8], want: &[u8], what: &str) {
    for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
        assert_eq!(
            g, w,
            "{what}：字节偏移 {i} 不同（得 0x{g:02X}，期望 0x{w:02X}）\n\
             得到：{got:02X?}\n期望：{want:02X?}"
        );
    }
    assert_eq!(
        got.len(),
        want.len(),
        "{what}：长度不同（得 {}，期望 {}）\n得到：{got:02X?}",
        got.len(),
        want.len()
    );
}

// ---------------------------------------------------------------- Setup

#[test]
fn setup_request_layout() {
    let got = encode_setup_request(b"MIT-MAGIC-COOKIE-1", &[0xDE, 0xAD, 0xBE, 0xEF]);
    #[rustfmt::skip]
    let want: &[u8] = &[
        // byte-order 'l'，unused
        b'l', 0x00,
        // protocol-major = 11，protocol-minor = 0
        0x0B, 0x00, 0x00, 0x00,
        // auth-name 长度 = 18（"MIT-MAGIC-COOKIE-1"）
        0x12, 0x00,
        // auth-data 长度 = 4
        0x04, 0x00,
        // unused
        0x00, 0x00,
        // "MIT-MAGIC-COOKIE-1"（18 字节） + 2 字节补齐到 4 的倍数
        b'M', b'I', b'T', b'-', b'M', b'A', b'G', b'I', b'C', b'-', b'C', b'O', b'O', b'K', b'I', b'E', b'-', b'1',
        0x00, 0x00,
        // auth-data（4 字节，无需补齐）
        0xDE, 0xAD, 0xBE, 0xEF,
    ];
    assert_bytes(&got, want, "Setup");
    assert_eq!(got.len(), 36, "12 字节头 + 20 字节名 + 4 字节数据");
}

#[test]
fn setup_request_without_auth_has_no_padding() {
    // 无认证：12 字节头就是全部（这是 Xvfb 默认 `-nolisten` 场景的最小包）
    let got = encode_setup_request(b"", b"");
    #[rustfmt::skip]
    let want: &[u8] = &[b'l', 0, 0x0B, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_bytes(&got, want, "Setup(empty)");
}

// ---------------------------------------------------------------- InternAtom

#[test]
fn intern_atom_pads_name_to_4_bytes() {
    // "WM_NAME" = 7 字节 → 补 1 字节 = 8 字节 = 2 个 4 字节单位
    // 请求长度 = 2（头）+ 2（名字）= 4 个单位
    let got = encode_intern_atom(b"WM_NAME", false);
    #[rustfmt::skip]
    let want: &[u8] = &[
        0x10, 0x00,       // opcode 16, only-if-exists = false
        0x04, 0x00,       // length = 4
        0x07, 0x00,       // name length = 7
        0x00, 0x00,       // unused
        b'W', b'M', b'_', b'N', b'A', b'M', b'E',
        0x00,             // padding
    ];
    assert_bytes(&got, want, "InternAtom(WM_NAME)");
}

#[test]
fn intern_atom_with_aligned_name_has_no_padding() {
    // "UTF8_STRING" = 11 字节 → 补 1；换成 8 字节的 "WM_CLASS" 验证"无需补齐"的分支
    let got = encode_intern_atom(b"WM_CLASS", true);
    assert_eq!(got[1], 1, "only-if-exists 应为 1");
    assert_eq!(&got[2..4], &[0x04, 0x00], "8 字节名字 → 长度 2+2=4");
    assert_eq!(got.len(), 8 + 8, "头 8 字节 + 名字 8 字节，无补齐");
    assert_eq!(&got[8..], b"WM_CLASS");
}

// ---------------------------------------------------------------- CreateWindow

const CREATE_VALUES: &[(ValueMask, u32)] = &[
    (ValueMask::BackPixel, 0x00FF_FFFF),
    (ValueMask::EventMask, 0x0000_0003),
];

#[test]
fn create_window_value_mask_and_list_order() {
    let got = encode_create_window(&CreateWindowArgs {
        wid: 0x00A0_0001,
        parent: 0x0000_02A0,
        x: 10,
        y: 20,
        width: 800,
        height: 600,
        border_width: 0,
        class: WindowClass::InputOutput,
        visual: 0, // CopyFromParent
        values: CREATE_VALUES,
    });
    #[rustfmt::skip]
    let want: &[u8] = &[
        0x01, 0x00,             // opcode 1, depth = CopyFromParent
        0x0A, 0x00,             // length = 8 + 2 个 value = 10
        0x01, 0x00, 0xA0, 0x00, // wid   = 0x00A00001
        0xA0, 0x02, 0x00, 0x00, // parent= 0x000002A0
        0x0A, 0x00,             // x = 10
        0x14, 0x00,             // y = 20
        0x20, 0x03,             // width  = 800
        0x58, 0x02,             // height = 600
        0x00, 0x00,             // border-width = 0
        0x01, 0x00,             // class = InputOutput(1)
        0x00, 0x00, 0x00, 0x00, // visual = CopyFromParent
        // value-mask = BackPixel(bit1) | EventMask(bit11) = 0x0802
        // 【真服务器纠正】我一开始把 EventMask 当成 bit10（那是 SaveUnder），
        // 导致 CreateWindow 报 BadValue；规范里 EventMask 是 bit 11。
        0x02, 0x08, 0x00, 0x00,
        // value-list：必须按位升序 → BackPixel 在前，EventMask 在后
        0xFF, 0xFF, 0xFF, 0x00, // BackPixel = 0x00FFFFFF
        0x03, 0x00, 0x00, 0x00, // EventMask = 0x3
    ];
    assert_bytes(&got, want, "CreateWindow");
}

#[test]
fn create_window_sorts_values_by_mask_bit() {
    // 故意**倒序**传入：EventMask 的位(10) 比 BackPixel(1) 高
    // 若实现不排序，value-list 就会写反 → 值填到错误属性上
    const REVERSED: &[(ValueMask, u32)] = &[
        (ValueMask::EventMask, 0x0000_0003),
        (ValueMask::BackPixel, 0x00FF_FFFF),
    ];
    let got = encode_create_window(&CreateWindowArgs {
        wid: 0x1A0_0001,
        parent: 0x2A0,
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        border_width: 0,
        class: WindowClass::InputOutput,
        visual: 0,
        values: REVERSED,
    });
    // 排序后的顺序应与正序传入完全一致
    let sorted = encode_create_window(&CreateWindowArgs {
        wid: 0x1A0_0001,
        parent: 0x2A0,
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        border_width: 0,
        class: WindowClass::InputOutput,
        visual: 0,
        values: CREATE_VALUES,
    });
    assert_bytes(&got, &sorted, "CreateWindow(倒序传入应被排序)");
}

#[test]
fn create_window_without_values_has_no_value_list() {
    let got = encode_create_window(&CreateWindowArgs {
        wid: 1,
        parent: 2,
        x: 0,
        y: 0,
        width: 100,
        height: 50,
        border_width: 1,
        class: WindowClass::InputOnly,
        visual: 0,
        values: &[],
    });
    assert_eq!(got.len(), 32, "无 value 时包长 32 字节");
    assert_eq!(&got[2..4], &[0x08, 0x00], "length = 8");
    assert_eq!(&got[22..24], &[0x02, 0x00], "class = InputOnly(2)");
    assert_eq!(&got[28..32], &[0, 0, 0, 0], "value-mask = 0");
}

// ---------------------------------------------------------------- MapWindow

#[test]
fn map_window_is_exactly_8_bytes() {
    let got = encode_map_window(0x00A0_0001);
    #[rustfmt::skip]
    let want: &[u8] = &[
        0x08, 0x00,             // opcode 8, unused
        0x02, 0x00,             // length = 2
        0x01, 0x00, 0xA0, 0x00, // window
    ];
    assert_bytes(&got, want, "MapWindow");
}

// ---------------------------------------------------------------- PutImage

#[test]
fn put_image_zpixmap_stride_and_length() {
    // 2x1 像素，BGRA 每像素 4 字节 → 8 字节数据，2 个单位
    // 请求长度 = 6（头）+ 2（数据）= 8
    let data: &[u8] = &[0x10, 0x20, 0x30, 0xFF, 0x40, 0x50, 0x60, 0xFF];
    let got = encode_put_image(&PutImageArgs {
        drawable: 0x00A0_0001,
        gc: 0x00A0_0002,
        width: 2,
        height: 1,
        dst_x: 0,
        dst_y: 0,
        depth: 24,
        data,
    });
    #[rustfmt::skip]
    let want: &[u8] = &[
        0x48, 0x02,             // opcode 72, format = ZPixmap(2)
        0x08, 0x00,             // length = 8
        0x01, 0x00, 0xA0, 0x00, // drawable
        0x02, 0x00, 0xA0, 0x00, // gc
        0x02, 0x00,             // width = 2
        0x01, 0x00,             // height = 1
        0x00, 0x00,             // dst-x = 0
        0x00, 0x00,             // dst-y = 0
        0x00,                   // left-pad = 0
        0x18,                   // depth = 24
        0x00, 0x00,             // unused
        0x10, 0x20, 0x30, 0xFF, // pixel 0
        0x40, 0x50, 0x60, 0xFF, // pixel 1
    ];
    assert_bytes(&got, want, "PutImage");
    assert_eq!(ImageFormat::ZPixmap as u8, 2);
}

#[test]
fn put_image_data_is_padded_to_4_bytes() {
    // 1x1 像素但数据只给 3 字节（理论上不该发生，但要验证补齐逻辑）
    let got = encode_put_image(&PutImageArgs {
        drawable: 1,
        gc: 2,
        width: 1,
        height: 1,
        dst_x: 0,
        dst_y: 0,
        depth: 24,
        data: &[1, 2, 3],
    });
    assert_eq!(got.len(), 24 + 4, "数据 3 字节应补到 4");
    assert_eq!(&got[2..4], &[0x07, 0x00], "length = 6 + 1 = 7");
    assert_eq!(&got[24..28], &[1, 2, 3, 0], "补齐应为 0");
}

/// **关键发现：`PutImage` 的 `length` 是 u16，单包最大只能送 ~256 KB 像素。**
///
/// 算法：`length` 以 4 字节为单位，上限 65535 → 数据上限 65529 单位 = 262,116 字节。
/// 对 BGRA（4 字节/像素）而言：**最大 262,116 / 4 = 65,529 个像素**（约 255×256）。
///
/// 结论：1080p 必须**分块**发送（约 40 个块）。这不是 bug，是 X11 协议的硬限制，
/// 也是 S3 必须上 MIT-SHM 的另一个理由（SHM 只传句柄，不受此限制）。
#[test]
fn put_image_length_field_caps_single_request_at_256kb() {
    let max_units = u16::MAX as usize; // 65535
    let max_data_units = max_units - 6; // 去掉 6 个单位的头
    let max_bytes = max_data_units * 4;
    assert_eq!(max_bytes, 262_116);
    assert_eq!(max_bytes / 4, 65_529, "最大像素数");

    // 1920x1080 单帧：8,294,400 字节 → 2,073,606 个单位 ≫ 65535，必须分块
    let full = 6 + (1920usize * 1080 * 4) / 4;
    assert!(full > max_units, "1080p 单帧需 {full} 个单位，超上限 {max_units}");
    assert_eq!(
        full.div_ceil(max_units - 6),
        32,
        "最少需要 32 个块（每块 ≤ 262,116 字节）"
    );

    // 255x256 = 65,280 像素 = 261,120 字节 → 在限制内
    let fits = 6 + (255usize * 256 * 4) / 4;
    assert!(fits <= max_units, "255x256 在限制内（{fits} ≤ {max_units}）");

    // 256x256 = 65,536 像素 = 262,144 字节 → 就差那么一点点
    let over = 6 + (256usize * 256 * 4) / 4;
    assert!(over > max_units, "256x256 已超限（{over} > {max_units}）");
}

// ---------------------------------------------------------------- ChangeProperty

#[test]
fn change_property_format8_counts_items_not_bytes() {
    let got = encode_change_property(0x00A0_0001, 39, 31, 8, PropMode::Replace, b"Y901");
    #[rustfmt::skip]
    let want: &[u8] = &[
        0x12, 0x00,             // opcode 18, mode = Replace(0)
        0x07, 0x00,             // length = 6 + 1 = 7
        0x01, 0x00, 0xA0, 0x00, // window
        0x27, 0x00, 0x00, 0x00, // property = 39
        0x1F, 0x00, 0x00, 0x00, // type = 31 (STRING)
        0x08,                   // format = 8
        0x00, 0x00, 0x00,       // unused
        0x04, 0x00, 0x00, 0x00, // data length = 4 **项**（格式 8 时等于字节数）
        b'Y', b'9', b'0', b'1',
    ];
    assert_bytes(&got, want, "ChangeProperty(format=8)");
}

#[test]
fn change_property_format32_divides_bytes_to_items() {
    // format=32 时，data length 是**项目数** = 字节数 / 4。弄错会把标题写成乱码。
    let data: &[u8] = &[1, 0, 0, 0, 2, 0, 0, 0]; // 两个 u32 → 2 项
    let got = encode_change_property(1, 2, 3, 32, PropMode::Replace, data);
    assert_eq!(&got[20..24], &[0x02, 0x00, 0x00, 0x00], "应写 2 项而不是 8 字节");
    assert_eq!(&got[2..4], &[0x08, 0x00], "length = 6 + 2 = 8");
}

// ---------------------------------------------------------------- poll 等价物

#[test]
fn get_input_focus_forces_a_round_trip() {
    let got = encode_get_input_focus();
    assert_bytes(&got, &[0x2B, 0x00, 0x01, 0x00], "GetInputFocus");
}

#[test]
fn no_operation_packet() {
    let got = encode_no_operation();
    assert_bytes(&got, &[0x7F, 0x00, 0x01, 0x00], "NoOperation");
}

// ---------------------------------------------------------------- 补齐工具

#[test]
fn pad4_aligns_upward() {
    assert_eq!(pad4(0), 0);
    assert_eq!(pad4(1), 4);
    assert_eq!(pad4(4), 4);
    assert_eq!(pad4(5), 8);
    assert_eq!(pad4(7), 8);
    assert_eq!(pad4(8), 8);
}

// ---------------------------------------------------------------- 解码器

/// 构造一个 32 字节的事件，并在指定偏移写入字段。
fn event32(code: u8, byte1: u8, fields: &[(usize, &[u8])]) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[0] = code;
    e[1] = byte1;
    for (off, bytes) in fields {
        e[*off..*off + bytes.len()].copy_from_slice(bytes);
    }
    e
}

#[test]
fn decode_expose() {
    // Expose：0=12, 1=unused, 2-3=sequence, 4-7=window, 8-9=x, 10-11=y, 12-13=w, 14-15=h, 16-17=count
    let e = event32(
        12,
        0,
        &[
            (2, &[0x01, 0x00]),
            (4, &[0x01, 0x00, 0xA0, 0x00]),
            (8, &[0x10, 0x00]),
            (10, &[0x20, 0x00]),
            (12, &[0xE0, 0x01]), // 480
            (14, &[0x1A, 0x01]), // 282
            (16, &[0x00, 0x00]),
        ],
    );
    let got = decode_event(&e).expect("解码");
    assert_eq!(
        got,
        X11Event::Expose {
            window: 0x00A0_0001,
            x: 0x10,
            y: 0x20,
            width: 0x1E0,
            height: 0x11A,
            count: 0,
        }
    );
}

#[test]
fn decode_configure_notify() {
    // ConfigureNotify：0=22, 4-7=event, 8-11=window, 12-15=above-sibling,
    // 16-17=x, 18-19=y, 20-21=w, 22-23=h, 24-25=border, 26=override-redirect
    let e = event32(
        22,
        0,
        &[
            (4, &[0x01, 0x00, 0xA0, 0x00]),
            (8, &[0x01, 0x00, 0xA0, 0x00]),
            (12, &[0x00, 0x00, 0x00, 0x00]),
            (16, &[0x0A, 0x00]),
            (18, &[0x14, 0x00]),
            (20, &[0x20, 0x03]), // 800
            (22, &[0x58, 0x02]), // 600
            (24, &[0x00, 0x00]),
            (26, &[0x01]),
        ],
    );
    let got = decode_event(&e).expect("解码");
    assert_eq!(
        got,
        X11Event::ConfigureNotify {
            event: 0x00A0_0001,
            window: 0x00A0_0001,
            above_sibling: 0,
            x: 10,
            y: 20,
            width: 800,
            height: 600,
            border_width: 0,
            override_redirect: true,
        }
    );
}

#[test]
fn decode_key_press_and_release() {
    // KeyPress：0=2, 1=keycode, 2-3=seq, 4-7=time, 8-11=root, 12-15=event, 16-19=child,
    // 20-21=root-x, 22-23=root-y, 24-25=event-x, 26-27=event-y, 28-29=state, 30=same-screen
    let fields: &[(usize, &[u8])] = &[
        (12, &[0x01, 0x00, 0xA0, 0x00]),
        (24, &[0x64, 0x00]), // 100
        (26, &[0xC8, 0x00]), // 200
        (28, &[0x04, 0x00]), // Ctrl 位
    ];
    let press = event32(2, 0x26, fields); // keycode 0x26 = 't'
    assert_eq!(
        decode_event(&press).unwrap(),
        X11Event::Key {
            pressed: true,
            keycode: 0x26,
            event: 0x00A0_0001,
            state: 4,
            event_x: 100,
            event_y: 200,
        }
    );
    let release = event32(3, 0x26, fields);
    assert!(matches!(
        decode_event(&release).unwrap(),
        X11Event::Key { pressed: false, .. }
    ));
}

#[test]
fn decode_button_press() {
    let fields: &[(usize, &[u8])] = &[
        (12, &[0x01, 0x00, 0xA0, 0x00]),
        (24, &[0x0A, 0x00]),
        (26, &[0x14, 0x00]),
        (28, &[0x00, 0x00]),
    ];
    let e = event32(4, 1, fields); // button 1 = 左键
    assert_eq!(
        decode_event(&e).unwrap(),
        X11Event::Button {
            pressed: true,
            button: 1,
            event: 0x00A0_0001,
            state: 0,
            event_x: 10,
            event_y: 20,
        }
    );
}

#[test]
fn decode_client_message_delete_window() {
    // ClientMessage：0=33, 1=format, 4-7=window, 8-11=type, 12-31=data[5]
    let e = event32(
        33,
        32,
        &[
            (4, &[0x01, 0x00, 0xA0, 0x00]),
            (8, &[0xAB, 0x02, 0x00, 0x00]), // WM_PROTOCOLS
            (12, &[0xAC, 0x02, 0x00, 0x00]), // WM_DELETE_WINDOW
            (16, &[0x00, 0x00, 0x00, 0x00]),
        ],
    );
    assert_eq!(
        decode_event(&e).unwrap(),
        X11Event::ClientMessage {
            window: 0x00A0_0001,
            message_type: 0x2AB,
            format: 32,
            data: [0x2AC, 0, 0, 0, 0],
        }
    );
}

#[test]
fn decode_motion_notify() {
    let fields: &[(usize, &[u8])] = &[
        (12, &[0x01, 0x00, 0xA0, 0x00]),
        (24, &[0x05, 0x00]),
        (26, &[0x06, 0x00]),
        (28, &[0x00, 0x00]),
    ];
    let e = event32(6, 0, fields);
    assert_eq!(
        decode_event(&e).unwrap(),
        X11Event::Motion {
            event: 0x00A0_0001,
            state: 0,
            event_x: 5,
            event_y: 6,
        }
    );
}

#[test]
fn decode_masks_synthetic_bit() {
    // 事件字节 0 的最高位是 "send_event" 标志；不屏蔽就会把 KeyPress 认成未知事件
    let mut e = event32(2, 0x26, &[(12, &[0x01, 0x00, 0xA0, 0x00])]);
    e[0] |= 0x80;
    assert!(matches!(
        decode_event(&e).unwrap(),
        X11Event::Key { pressed: true, .. }
    ));
}

#[test]
fn decode_short_buffer_is_an_error_not_a_panic() {
    assert_eq!(
        decode_event(&[0u8; 31]),
        Err(DecodeError::TooShort { got: 31 })
    );
}

#[test]
fn decode_unknown_event_is_preserved() {
    let e = event32(99, 0, &[]);
    assert_eq!(decode_event(&e).unwrap(), X11Event::Unknown { code: 99 });
}
