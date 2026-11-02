//! 截图：把 `Host::on_snapshot` 回传的 BGRA 像素写成 PNG。
//!
//! 为什么单独一个模块而不放进主循环：截图是**应用层的决定**
//! （何时截、截到哪里、截完要不要退出）；
//! 框架只保证"能拿到一帧真实像素"。

use std::io::Write;

/// 把 BGRA 像素写成 PNG 文件。
///
/// 输入：路径、`w*h*4` 字节的 BGRA、尺寸。
/// 输出：`Err` 带可读原因（目录不存在、字节数不足）。
/// 副作用：写磁盘。
///
/// 为什么要换通道：D3D/X11 回读出来是 **BGRA**，PNG 要的是 **RGBA**。
/// 不换的话红蓝会对调（看起来像“颜色变了”，实际是通道顺序错了）。
pub fn write_bgra_png(
    path: &std::path::Path,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<(), String> {
    let need = (width as usize) * (height as usize) * 4;
    if width == 0 || height == 0 || pixels.len() < need {
        return Err(format!(
            "像素数据不足：{width}x{height} 需要 {need} 字节，实际 {}",
            pixels.len()
        ));
    }
    let mut rgba = Vec::with_capacity(need);
    for px in pixels[..need].chunks_exact(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }

    let file = std::fs::File::create(path).map_err(|e| format!("创建文件失败: {e}"))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| format!("PNG 头写入失败: {e}"))?;
    writer
        .write_image_data(&rgba)
        .map_err(|e| format!("像素写入失败: {e}"))?;
    writer
        .finish()
        .map_err(|e| format!("PNG 收尾失败: {e}"))?;
    let _ = std::io::stdout().flush();
    Ok(())
}
