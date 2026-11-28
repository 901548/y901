//! **X11 请求编码的结构不变量**（属性测试，确定性伪随机）。
//!
//! 不解析协议，只检查**服务端收到包后第一件会做的事**：
//!
//! 不解析协议，只检查**任何服务端都会先检查的那几件事**：
//! 长度字段、4 字节对齐、opcode、以及"大 payload 必须被拒绝而不是静默回绕"。

use y901_backend_x11::encode;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn u32(&mut self) -> u32 {
        self.next() as u32
    }
    fn u16(&mut self) -> u16 {
        self.next() as u16
    }
}

/// 所有请求的共同不变量：整包 4 字节对齐；`length` 字段（字节 2..4，小端）
/// 必须等于 `总字节 / 4`，且不超协议上限。
fn check_header(bytes: &[u8], http: &str) {
    assert!(
        bytes.len() >= 4,
        "[{http}] 请求至少 4 字节头，实得 {}",
        bytes.len()
    );
    assert_eq!(
        bytes.len() % 4,
        0,
        "[{http}] 请求必须 4 字节对齐，实得 {} 字节",
        bytes.len()
    );
    let len_field = u16::from_le_bytes([bytes[2], bytes[3]]) as usize;
    assert_eq!(
        len_field * 4,
        bytes.len(),
        "[{http}] length 字段 {} (= {} 字节) 与实际长度 {} 不一致",
        len_field,
        len_field * 4,
        bytes.len()
    );
    assert!(
        bytes.len() <= encode::MAX_REQUEST_BYTES,
        "[{http}] 超过单请求上限"
    );
}

#[test]
fn every_small_request_has_a_consistent_length_field() {
    let mut rng = Lcg(0x1122_3344);
    for case in 0..300 {
        let mut reqs: Vec<(String, Vec<u8>)> = Vec::new();
        reqs.push((
            "intern_atom".to_string(),
            encode::encode_intern_atom(b"WM_PROTOCOLS", rng.next().is_multiple_of(2)),
        ));
        reqs.push((
            "map_window".to_string(),
            encode::encode_map_window(rng.u32()),
        ));
        reqs.push((
            "destroy_window".to_string(),
            encode::encode_destroy_window(rng.u32()),
        ));
        reqs.push(("free_gc".to_string(), encode::encode_free_gc(rng.u32())));
        reqs.push((
            "change_property".to_string(),
            encode::encode_change_property(
                rng.u32(),
                rng.u32(),
                rng.u32(),
                8,
                encode::PropMode::Replace,
                b"hello",
            ),
        ));
        reqs.push((
            "get_property".to_string(),
            encode::encode_get_property(
                rng.u32(),
                rng.u32(),
                rng.u32(),
                rng.u32(),
                rng.u32(),
                false,
            ),
        ));
        reqs.push((
            "get_image".to_string(),
            encode::encode_get_image(
                rng.u32(),
                0,
                0,
                rng.u16() % 64 + 1,
                rng.u16() % 64 + 1,
                0xFFFFFFFF,
            ),
        ));
        reqs.push((
            "get_input_focus".to_string(),
            encode::encode_get_input_focus(),
        ));
        reqs.push(("no_operation".to_string(), encode::encode_no_operation()));
        reqs.push((
            "query_extension".to_string(),
            encode::encode_query_extension(b"SHAPE"),
        ));
        reqs.push((
            "get_keyboard_mapping".to_string(),
            encode::encode_get_keyboard_mapping(8, 248),
        ));

        for (name, bytes) in reqs {
            check_header(&bytes, &format!("case{case}/{name}"));
        }
    }
}

#[test]
fn put_image_length_matches_payload() {
    let mut rng = Lcg(0x5566_7788);
    for _ in 0..200 {
        let w = rng.u16() % 40 + 1;
        let h = rng.u16() % 40 + 1;
        let data = vec![0xABu8; (w as usize) * (h as usize) * 4];
        let args = encode::PutImageArgs {
            drawable: rng.u32(),
            gc: rng.u32(),
            width: w,
            height: h,
            dst_x: 0,
            dst_y: 0,
            depth: 24,
            data: &data,
        };
        let bytes = encode::encode_put_image(&args);
        check_header(&bytes, "put_image");
        // 头部 24 字节 + payload
        assert_eq!(
            bytes.len(),
            24 + data.len(),
            "PutImage 总长应为头 + 数据（{w}x{h}）"
        );
    }
}

#[test]
fn oversized_payloads_are_rejected_not_wrapped() {
    // 超过单请求上限的数据：`request_fits` 必须为 false（调用方据此分块），
    // 而不是编码出一个回绕成小值的 length 字段。
    assert!(!encode::request_fits(encode::MAX_REQUEST_BYTES + 4));
    assert!(encode::request_fits(encode::MAX_REQUEST_BYTES));
    // 非 4 对齐也必须判"装不下"（否则 length 会截断）
    assert!(!encode::request_fits(encode::MAX_REQUEST_BYTES - 2));

    // PutImage 的数据上限与协议一致：6 个单位头（24 字节）
    assert_eq!(encode::PUT_IMAGE_MAX_BYTES, (u16::MAX as usize - 6) * 4);
    assert_eq!(encode::PUT_IMAGE_MAX_BYTES + 24, encode::MAX_REQUEST_BYTES);
}
