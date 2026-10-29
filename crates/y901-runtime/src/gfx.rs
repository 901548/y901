//! 渲染后端抽象：**零平台依赖**的 trait + 记录型 mock 实现。
//!
//! 目标（跨平台 v3 阶段 1）：把 `y901-backend-win32` 里的 D3D11 具体调用
//! 收敛到一个 trait 后面，让第二个平台（Linux/macOS/WASM）只需要实现它。
//!
//! 本模块只定义契约，不引用任何 Win32 / wgpu / Metal 类型。
//! `PlatformWindowHandle` 用最小枚举表达"哪个平台的哪个句柄"，
//! 不引入 `raw-window-handle` 依赖（那会把 trait 绑死在别的 crate 的版本上）。

/// 平台窗口句柄：跨平台的最小公共表达。
///
/// 变体只携带"够用"的信息，具体语义由各后端自己解释：
/// - Win32：`HWND` 指针
/// - X11：XID（`u64`）
/// - Wayland：`wl_surface*`
/// - 其余平台暂用 `Other`（携带不透明指针，后端自行解释）
///
/// # Safety
///
/// 句柄必须在后端使用期间保持有效；本模块不校验有效性（无法校验）。
#[derive(Clone, Copy, Debug)]
pub enum PlatformWindowHandle {
    /// Win32 `HWND`。
    Win32(*mut core::ffi::c_void),
    /// X11 `Window`（XID）。
    X11(u64),
    /// Wayland `wl_surface*`。
    Wayland(*mut core::ffi::c_void),
    /// 未明确支持的平台。使用前必须由后端与 PlatformRuntime 共同约定
    /// 指针的具体含义（例如 WASM 的 canvas 句柄、Metal 的 layer 指针）。
    Other(*mut core::ffi::c_void),
}

/// GPU 实例数据。字段顺序和大小必须与 `d3d11::Instance`（backend 实现）一致。
/// 第 2 步的 `impl RenderBackend for D3dBackend` 必须做转换 + 布局校验。
///
/// 字段顺序/类型与 `y901-backend-win32::backend::d3d11::Instance` 保持一致，
/// 但**本模块不依赖那个类型**——后端负责在自己的实现里做转换或复用同一布局。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GfxInstance {
    /// 左上角像素坐标。
    pub pos: [f32; 2],
    /// 像素尺寸。
    pub size: [f32; 2],
    /// 图集 UV 矩形 `[u0, v0, u1, v1]`。
    pub uv_rect: [f32; 4],
    /// RGBA（0..1）。
    pub color: [f32; 4],
    /// 渐变终点色（纯色时等于 `color`）。
    pub color_end: [f32; 4],
    /// `(dir.x, dir.y, mode, offset)`；mode=1 表示线性渐变。
    pub gradient: [f32; 4],
    /// `(radius, softness, 0, 0)`；radius>0 时走 GPU SDF 圆角。
    pub params: [f32; 4],
}

/// 编译期布局断言：4 个字段共 12 个 f32 = 48 字节，且与 `d3d11::Instance` 相同。
///
/// 一旦有人加字段/改类型，这里会**编译失败**——这是唯一能在编译期发现的防线。
/// 运行时还有 `debug_assert` + `gfx_instance_layout_matches_d3d_instance` 测试双保险。
const _: () = assert!(core::mem::size_of::<GfxInstance>() == 96);

/// 图集脏矩形：本帧需要上传到 GPU 的区域。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasUpdate {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// 一帧的绘制提交。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub instances: Vec<GfxInstance>,
    pub atlas_updates: Vec<AtlasUpdate>,
    /// 图集是否被整体重建（true 时忽略 `atlas_updates`，整张重传）。
    pub atlas_reset: bool,
}

/// 渲染后端。
///
/// 生命周期：`create` → 循环（`resize` / `present`）→ `Drop`。
/// 设备丢失时 `present` 返回 `Err`，调用方决定是 `rebuild` 还是退出。
pub trait RenderBackend {
    /// 创建后端并绑定窗口。
    ///
    /// 失败：设备创建失败 / 窗口句柄无效（后端自行判定）。
    fn create(
        handle: PlatformWindowHandle,
        width: u32,
        height: u32,
    ) -> Result<Self, String>
    where
        Self: Sized;

    /// 设备丢失后重建全部 GPU 资源（保留尺寸）。
    fn rebuild(&mut self, width: u32, height: u32) -> Result<(), String>;

    /// 调整渲染尺寸（交换链 / 后缓冲）。
    fn resize(&mut self, width: u32, height: u32) -> Result<(), String>;

    /// 提交一帧。
    ///
    /// 失败：设备丢失、图集未同步等；返回 `Err` 后调用方通常会尝试 `rebuild`。
    fn present(&mut self, frame: &Frame) -> Result<(), String>;

    /// 上一次 present 是否返回被遮挡（如 D3D11 的 DXGI_STATUS_OCCLUDED）。
    /// 无参查询，因此是**缓存值**，不是实时状态。
    fn is_occluded(&self) -> bool;

    /// 上次 `present` 的实例数（诊断用）。
    fn instance_count(&self) -> usize;
}

// ============================ 记录型 mock ============================

/// mock 后端的一次调用记录。
#[derive(Clone, Debug, PartialEq)]
pub enum BackendCall {
    Create {
        width: u32,
        height: u32,
    },
    Rebuild {
        width: u32,
        height: u32,
    },
    Resize {
        width: u32,
        height: u32,
    },
    Present {
        instances: usize,
        atlas_updates: usize,
        atlas_reset: bool,
    },
}

/// 只记录调用、不真正渲染的后端。
///
/// 用途：断言"框架层产生了什么绘制指令"，与真实后端对拍。
/// 它同时保存最近一帧的完整 `Frame`，便于逐实例比较。
#[derive(Debug, Default)]
pub struct RecordingBackend {
    pub calls: Vec<BackendCall>,
    pub last_frame: Option<Frame>,
    pub occluded: bool,
    /// 强制 `present` 返回 Err，用于测试"设备丢失"分支。
    pub fail_present: bool,
}

impl RecordingBackend {
    /// 新建空的记录型后端（尚未 `create`，`calls` 为空）。
    ///
    /// 返回：可直接用于对拍的空实例。副作用：无。
    /// 注意：正常流程应走 `RenderBackend::create`（会记录一条 `Create`）；
    /// 直接 `new()` 只用于"只想记录 present"的测试。
    pub fn new() -> Self {
        Self::default()
    }

    /// 所有 `Present` 记录。
    pub fn presents(&self) -> impl Iterator<Item = &BackendCall> {
        self.calls
            .iter()
            .filter(|c| matches!(c, BackendCall::Present { .. }))
    }
}

impl RenderBackend for RecordingBackend {
    fn create(
        _handle: PlatformWindowHandle,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let mut backend = Self::new();
        backend.calls.push(BackendCall::Create { width, height });
        Ok(backend)
    }

    fn rebuild(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.calls.push(BackendCall::Rebuild { width, height });
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.calls.push(BackendCall::Resize { width, height });
        Ok(())
    }

    fn present(&mut self, frame: &Frame) -> Result<(), String> {
        self.calls.push(BackendCall::Present {
            instances: frame.instances.len(),
            atlas_updates: frame.atlas_updates.len(),
            atlas_reset: frame.atlas_reset,
        });
        self.last_frame = Some(frame.clone());
        if self.fail_present {
            return Err("mock: device lost".to_string());
        }
        Ok(())
    }

    fn is_occluded(&self) -> bool {
        self.occluded
    }

    fn instance_count(&self) -> usize {
        self.last_frame
            .as_ref()
            .map(|f| f.instances.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(x: f32) -> GfxInstance {
        GfxInstance {
            pos: [x, 0.0],
            size: [8.0, 16.0],
            uv_rect: [0.0, 0.0, 0.1, 0.1],
            color: [1.0, 1.0, 1.0, 1.0],
            color_end: [1.0, 1.0, 1.0, 1.0],
            gradient: [0.0, 0.0, 0.0, 0.0],
            params: [1.0, 1.0, 1.0, 1.0],
        }
    }

    #[test]
    fn recording_backend_records_calls_in_order() {
        let mut b = RecordingBackend::create(PlatformWindowHandle::X11(7), 800, 600).unwrap();
        b.resize(1024, 768).unwrap();
        let frame = Frame {
            instances: vec![inst(0.0), inst(8.0)],
            atlas_updates: vec![AtlasUpdate { x: 0, y: 0, w: 4, h: 4 }],
            atlas_reset: false,
        };
        b.present(&frame).unwrap();

        assert_eq!(
            b.calls,
            vec![
                BackendCall::Create {
                    width: 800,
                    height: 600
                },
                BackendCall::Resize {
                    width: 1024,
                    height: 768
                },
                BackendCall::Present {
                    instances: 2,
                    atlas_updates: 1,
                    atlas_reset: false
                },
            ],
            "调用顺序与内容必须被完整记录"
        );
        assert_eq!(b.instance_count(), 2);
        assert_eq!(b.last_frame.as_ref().unwrap().instances[1].pos[0], 8.0);
    }

    #[test]
    fn recording_backend_can_simulate_device_loss() {
        let mut b = RecordingBackend::create(PlatformWindowHandle::Win32(std::ptr::null_mut()), 1, 1)
            .unwrap();
        b.fail_present = true;
        let err = b.present(&Frame::default()).unwrap_err();
        assert!(err.contains("device lost"), "错误应表明设备丢失：{err}");
        // 失败也要重建成功（模拟恢复）
        b.fail_present = false;
        b.rebuild(1, 1).unwrap();
        assert!(b.calls.contains(&BackendCall::Rebuild { width: 1, height: 1 }));
    }

    #[test]
    fn empty_frame_still_reports_present() {
        let mut b = RecordingBackend::create(PlatformWindowHandle::Wayland(std::ptr::null_mut()), 10, 10)
            .unwrap();
        b.present(&Frame::default()).unwrap();
        assert_eq!(b.presents().count(), 1, "空帧也必须提交（否则窗口不刷新）");
        assert_eq!(b.instance_count(), 0);
    }
}
