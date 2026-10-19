//! 基础几何类型。
//!
//! 所有整数坐标都是**物理像素**，原点在虚拟桌面左上角、y 轴向下。这是 Windows 的
//! 原生约定，也是 `spotlight-core` 内部唯一的表示；macOS 的左下原点由平台层在边界
//! 上翻转，不进到这里。

/// 整数矩形，左闭右开：`[x, x+w) × [y, y+h)`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const ZERO: Rect = Rect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };

    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn from_edges(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            x: left,
            y: top,
            w: right - left,
            h: bottom - top,
        }
    }

    pub const fn right(self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(self) -> i32 {
        self.y + self.h
    }

    pub const fn is_empty(self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn area(self) -> i64 {
        if self.is_empty() {
            0
        } else {
            i64::from(self.w) * i64::from(self.h)
        }
    }

    pub fn contains_point(self, x: i32, y: i32) -> bool {
        !self.is_empty() && x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    pub fn contains_rect(self, other: Rect) -> bool {
        !other.is_empty()
            && other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    /// 交集；不相交或任一为空时返回 `None`。
    pub fn intersect(self, other: Rect) -> Option<Rect> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= left || bottom <= top {
            None
        } else {
            Some(Rect::from_edges(left, top, right, bottom))
        }
    }

    pub fn intersects(self, other: Rect) -> bool {
        self.intersect(other).is_some()
    }

    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Rect::from_edges(
            self.x.min(other.x),
            self.y.min(other.y),
            self.right().max(other.right()),
            self.bottom().max(other.bottom()),
        )
    }

    /// 四边同时外扩 `d`（可为负，表示内缩）。
    pub fn inflate(self, d: i32) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + d * 2, self.h + d * 2)
    }

    pub fn translated(self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// 保持尺寸整体平移进 `bounds`；`bounds` 比自身小时贴左上并收缩。
    pub fn clamp_into(self, bounds: Rect) -> Rect {
        if bounds.is_empty() {
            return Rect::new(bounds.x, bounds.y, 0, 0);
        }
        let w = self.w.min(bounds.w);
        let h = self.h.min(bounds.h);
        let max_x = (bounds.right() - w).max(bounds.x);
        let max_y = (bounds.bottom() - h).max(bounds.y);
        Rect::new(
            self.x.clamp(bounds.x, max_x),
            self.y.clamp(bounds.y, max_y),
            w,
            h,
        )
    }

    /// 保持尺寸，只平移位置让它落进 `bounds`；`bounds` 比自身小时贴左上，不收缩。
    pub fn shift_into(self, bounds: Rect) -> Rect {
        if bounds.is_empty() {
            return self;
        }
        let max_x = (bounds.right() - self.w).max(bounds.x);
        let max_y = (bounds.bottom() - self.h).max(bounds.y);
        Rect::new(
            self.x.clamp(bounds.x, max_x),
            self.y.clamp(bounds.y, max_y),
            self.w,
            self.h,
        )
    }

    /// 落在 `bounds` 之外的部分的面积。
    pub fn outside_area(self, bounds: Rect) -> i64 {
        self.area() - self.intersect(bounds).map_or(0, Rect::area)
    }
}

/// 浮点矩形，供效果层使用（不参与坐标换算）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub const ZERO: RectF = RectF {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    };

    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn from_rect(r: Rect) -> Self {
        Self {
            x: r.x as f32,
            y: r.y as f32,
            w: r.w as f32,
            h: r.h as f32,
        }
    }

    pub fn right(self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub fn inflate(self, d: f32) -> Self {
        Self::new(self.x - d, self.y - d, self.w + d * 2.0, self.h + d * 2.0)
    }

    pub fn to_rect_round(self) -> Rect {
        Rect::from_edges(
            self.x.round() as i32,
            self.y.round() as i32,
            self.right().round() as i32,
            self.bottom().round() as i32,
        )
    }

    pub fn union(self, other: RectF) -> RectF {
        if self.w <= 0.0 || self.h <= 0.0 {
            return other;
        }
        if other.w <= 0.0 || other.h <= 0.0 {
            return self;
        }
        RectF::new(
            self.x.min(other.x),
            self.y.min(other.y),
            self.right().max(other.right()) - self.x.min(other.x),
            self.bottom().max(other.bottom()) - self.y.min(other.y),
        )
    }
}

/// 截图像素空间的矩形。用 newtype 与屏幕坐标隔离，让编译器拦住「拿图像像素当屏幕像素用」。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageRect(pub Rect);

impl ImageRect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self(Rect::new(x, y, w, h))
    }

    pub const fn from_rect(r: Rect) -> Self {
        Self(r)
    }

    pub const fn rect(self) -> Rect {
        self.0
    }
}

/// 屏幕物理像素空间的矩形。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScreenRect(pub Rect);

impl ScreenRect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self(Rect::new(x, y, w, h))
    }

    pub const fn from_rect(r: Rect) -> Self {
        Self(r)
    }

    pub const fn rect(self) -> Rect {
        self.0
    }

    pub fn inflate(self, d: i32) -> Self {
        Self(self.0.inflate(d))
    }

    pub fn translated(self, dx: i32, dy: i32) -> Self {
        Self(self.0.translated(dx, dy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_and_empty() {
        let r = Rect::from_edges(10, 20, 40, 60);
        assert_eq!((r.x, r.y, r.w, r.h), (10, 20, 30, 40));
        assert_eq!((r.right(), r.bottom()), (40, 60));
        assert!(!r.is_empty());
        assert!(Rect::new(0, 0, 0, 10).is_empty());
        assert_eq!(Rect::new(0, 0, 0, 10).area(), 0);
        assert_eq!(r.area(), 1200);
    }

    #[test]
    fn intersect_edge_cases() {
        let a = Rect::new(0, 0, 10, 10);
        // 边贴边不算相交（左闭右开）
        assert_eq!(a.intersect(Rect::new(10, 0, 10, 10)), None);
        assert!(!a.intersects(Rect::new(10, 0, 10, 10)));
        assert_eq!(
            a.intersect(Rect::new(5, 5, 10, 10)),
            Some(Rect::new(5, 5, 5, 5))
        );
        assert_eq!(a.intersect(Rect::new(-5, -5, 2, 2)), None);
    }

    #[test]
    fn contains_is_half_open() {
        let a = Rect::new(0, 0, 10, 10);
        assert!(a.contains_point(0, 0));
        assert!(a.contains_point(9, 9));
        assert!(!a.contains_point(10, 9));
        assert!(a.contains_rect(Rect::new(2, 2, 8, 8)));
        assert!(!a.contains_rect(Rect::new(2, 2, 9, 9)));
        // 空矩形不被任何矩形包含
        assert!(!a.contains_rect(Rect::ZERO));
    }

    #[test]
    fn union_with_empty_is_identity() {
        let a = Rect::new(1, 2, 3, 4);
        assert_eq!(a.union(Rect::ZERO), a);
        assert_eq!(Rect::ZERO.union(a), a);
        assert_eq!(
            Rect::new(0, 0, 10, 10).union(Rect::new(5, 5, 10, 10)),
            Rect::new(0, 0, 15, 15)
        );
    }

    #[test]
    fn clamp_into_keeps_size_when_possible() {
        let bounds = Rect::new(0, 0, 100, 100);
        let r = Rect::new(-20, -20, 30, 30);
        assert_eq!(r.clamp_into(bounds), Rect::new(0, 0, 30, 30));
        let r2 = Rect::new(90, 90, 30, 30);
        assert_eq!(r2.clamp_into(bounds), Rect::new(70, 70, 30, 30));
        // 比 bounds 还大：贴左上并收缩
        let big = Rect::new(-5, -5, 300, 300);
        assert_eq!(big.clamp_into(bounds), Rect::new(0, 0, 100, 100));
    }

    #[test]
    fn shift_into_keeps_size() {
        let bounds = Rect::new(0, 0, 100, 100);
        let r = Rect::new(-20, 50, 30, 30);
        assert_eq!(r.shift_into(bounds), Rect::new(0, 50, 30, 30));
        let r2 = Rect::new(90, 90, 30, 30);
        assert_eq!(r2.shift_into(bounds), Rect::new(70, 70, 30, 30));
        // 不收缩：比 bounds 还大时贴左上，尺寸不变
        assert_eq!(
            Rect::new(-5, -5, 300, 300).shift_into(bounds),
            Rect::new(0, 0, 300, 300)
        );
    }

    #[test]
    fn outside_area_counts_only_protrusion() {
        let bounds = Rect::new(0, 0, 100, 100);
        assert_eq!(Rect::new(0, 0, 50, 50).outside_area(bounds), 0);
        assert_eq!(Rect::new(90, 0, 20, 10).outside_area(bounds), 100);
        assert_eq!(Rect::new(200, 200, 10, 10).outside_area(bounds), 100);
    }

    #[test]
    fn rectf_round_trip() {
        let r = Rect::new(10, 20, 30, 40);
        assert_eq!(RectF::from_rect(r).to_rect_round(), r);
        assert_eq!(
            RectF::new(0.4, 0.6, 10.0, 10.0).to_rect_round(),
            Rect::new(0, 1, 10, 10)
        );
    }
}
