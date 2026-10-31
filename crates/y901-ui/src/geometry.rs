//! 几何原语。全部用 f32 逻辑像素，DPI 缩放由后端在变换里处理。

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pos {
    pub x: f32,
    pub y: f32,
}

impl Pos {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    pub const ZERO: Size = Size { w: 0.0, h: 0.0 };

    pub const fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }

    pub fn max(self, other: Size) -> Size {
        Size::new(self.w.max(other.w), self.h.max(other.h))
    }

    pub fn min(self, other: Size) -> Size {
        Size::new(self.w.min(other.w), self.h.min(other.h))
    }
}

/// 轴对齐矩形：左上角 + 尺寸。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    };

    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn from_size(size: Size) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: size.w,
            h: size.h,
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn size(&self) -> Size {
        Size::new(self.w, self.h)
    }

    pub fn contains(&self, p: Pos) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// 是否完全包含另一个矩形（边界相等也算包含）。
    ///
    /// 用途：脏区合并（被包含就不用再添加；包含了别人就删掉别人）。
    pub fn contains_rect(&self, other: Rect) -> bool {
        if other.is_empty() {
            return true;
        }
        if self.is_empty() {
            return false;
        }
        self.x <= other.x
            && self.y <= other.y
            && self.right() >= other.right()
            && self.bottom() >= other.bottom()
    }

    pub fn intersect(&self, other: Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        Rect::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }

    /// 内缩（padding）。
    /// 两个矩形的并集（包围盒）。空矩形按"不参与"处理。
    ///
    /// 返回：覆盖两者的最小矩形；两者都为空时返回空矩形。
    /// 用途：控件收缩/滚动时，旧区域和新区域**都要重绘**，
    /// 只重绘新区域会留下旧内容的残影。
    pub fn union(&self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return *self;
        }
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = self.right().max(other.right());
        let y1 = self.bottom().max(other.bottom());
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    pub fn inset(&self, edges: Edges) -> Rect {
        Rect::new(
            self.x + edges.left,
            self.y + edges.top,
            (self.w - edges.left - edges.right).max(0.0),
            (self.h - edges.top - edges.bottom).max(0.0),
        )
    }
}

/// 四边内边距。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub const ZERO: Edges = Edges {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };

    pub const fn all(v: f32) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    pub const fn symmetric(h: f32, v: f32) -> Self {
        Self {
            top: v,
            right: h,
            bottom: v,
            left: h,
        }
    }

    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

/// 把一个弹层（菜单/浮层）摆在锚点旁，**不选出父边界**。
///
/// 输入：`anchor`（触发控件的屏幕矩形）、`size`（弹层尺寸）、
/// `bounds`（允许的父区域）。
/// 返回：弹层应放的矩形。
///
/// 规则：
/// 1. 默认贴在锚点**下方**、左对齐；
/// 2. 下方放不下 → **向上翻转**（放在锚点上方）；
/// 3. 右侧放不下 → **向左平移**；
/// 4. 仍然超出父边界 → 夹到边界内（宁可贴边也不能超出）。
///
/// 为什么要抽成纯函数：命中矩形必须与绘制矩形**完全一致**，
/// 否则会出现"看不见但理论上可点"的区域；把摆放逻辑独立出来可以单测。
pub fn place_popup(anchor: Rect, size: Size, bounds: Rect) -> Rect {
    let w = size.w.min(bounds.w.max(0.0));
    let h = size.h.min(bounds.h.max(0.0));

    // 横向：先左对齐，放不下再左移，最后夹到边界
    let mut x = anchor.x;
    if x + w > bounds.right() {
        x = anchor.right() - w;
    }
    if x < bounds.x {
        x = bounds.x;
    }
    if x + w > bounds.right() {
        x = bounds.right() - w;
    }

    // 纵向：先在下方，放不下翻到上方，再夹到边界
    let mut y = anchor.bottom();
    if y + h > bounds.bottom() {
        y = anchor.y - h;
    }
    if y < bounds.y {
        y = bounds.y;
    }
    if y + h > bounds.bottom() {
        y = bounds.bottom() - h;
    }

    Rect::new(x, y, w, h)
}
