//! 守护进程：持有所有 overlay 窗口，串行处理 IPC 请求。
//!
//! 所有窗口操作都在主线程；IPC 线程只负责收包和回包。

use std::collections::VecDeque;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::json;
use spotlight_core::geom::{ImageRect, Rect, RectF, ScreenRect};
use spotlight_core::layout::{Placement, Preference, Size, place_preferring};
use spotlight_core::mapping::{CaptureId, CaptureMap, Segment};
use spotlight_core::mask;
use spotlight_core::monitor::monitor_for_rect;
use spotlight_core::protocol::{
    CaptureImage, CaptureInfo, CaptureParams, CaptureSummary, CapturesResult, ClearParams,
    ErrorBody, Method, MonitorInfo, RectInfo, ReloadResult, Request, Response, SceneSummary,
    SegmentInfo, ShowDebug, ShowParams, ShowResult, StatusResult, StepSpec, codes,
};
use spotlight_core::scene::{Callout, Scene, Target};
use spotlight_core::timing::{Timeout, resolve};
use spotlight_platform::win::effect::EffectWindow;
use spotlight_platform::win::hotkey;
use spotlight_platform::win::ipc::{self, Connection, Server};
use spotlight_platform::win::message::{self, Event, MessageBus};
use spotlight_platform::win::monitors;
use spotlight_platform::win::process::SingleInstance;
use spotlight_platform::win::shot;
use spotlight_platform::win::text::{TextBitmap, render_text};
use spotlight_platform::win::{
    MaskWindow, cursor_pos, enable_dpi_awareness, set_action_hit, set_stop_hit,
    take_action_request, take_dismiss_request, take_display_change,
};
use spotlight_render::{
    Badge, Button, Card, Ctx, Effect, EffectStyle, KNOWN_EFFECTS, Keycap, Progress, StopMark,
    TextLayer, build_effects, render_frame,
};

use crate::config::{self, Config, Theme};
use crate::paths;

/// 洞相对目标的 padding。要小于等于光环宽度，否则环外会露出一圈没压暗的亮边。
const PAD: i32 = 3;
const CARD_RADIUS: f32 = 10.0;
const CARD_PADDING: i32 = 12;
/// 标题行与正文之间的间距。
const TITLE_GAP: i32 = 4;
/// 右侧多留一点，避免文字顶到关闭叉。
const CARD_PADDING_RIGHT: i32 = 34;
const CARD_GAP: i32 = 12;

const CLOSE_MARK: i32 = 16;
const CLOSE_HIT: i32 = 28;
const CLOSE_MARGIN: i32 = CARD_PADDING;

const TICK_ACTIVE_MS: u32 = 100;
/// 检测点击的轮询间隔；静态画面也按这个节奏醒来。
const TICK_CLICK_MS: u32 = 30;
/// 入场动画期间用 60fps 的节奏。
const TICK_ANIM_MS: u32 = 16;
/// 入场淡入时长（毫秒）。
const ANIM_MS: f32 = 120.0;
const TICK_IDLE_MS: u32 = 5000;
const IDLE_EXIT: Duration = Duration::from_secs(180);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CAPTURES: usize = 20;
const DEFAULT_CAPTURE_WIDTH: u32 = 1280;

struct Job {
    req: Request,
    reply: mpsc::SyncSender<Response>,
}

/// 画面上各元素相对效果窗口的位置（客户区坐标）。
struct Geom {
    union: Rect,
    card_radius: f32,
    targets: Vec<Rect>,
    card: Option<Rect>,
    close: Option<Rect>,
    badges: Vec<(Rect, TextBitmap)>,
    keycap: Option<Rect>,
    keycap_text: Option<Rect>,
    /// 动作按钮：(胶囊, 文字)。
    button: Option<(Rect, Rect)>,
    title_at: Option<(f32, f32)>,
    text_at: Option<(f32, f32)>,
}

/// 卡片上那个按钮点了会发生什么。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    /// 还有下一步：跳到下一步。
    Next,
    /// 已经是最后一步：结束并告诉调用方"完成了"。
    Done,
}

struct Active {
    key: Option<String>,
    has_text: bool,
    targets: usize,
    total: Option<Duration>,
    remaining: Duration,
    last_tick: Instant,
    shown_at: Instant,
    paused: bool,
    paused_total: Duration,
    animate: bool,
    card_screen: Option<Rect>,
    geom: Geom,
    title: Option<TextBitmap>,
    text: Option<TextBitmap>,
    keycap: Option<TextBitmap>,
    button: Option<TextBitmap>,
    /// 按钮做的事：下一步，还是完成。
    action: Action,
    /// 高亮洞（窗口坐标）与它的圆角，用来补圆角外侧的遮罩。
    holes: Vec<Rect>,
    hole_radius: f32,
    dim: [u8; 4],
    effects: Vec<Box<dyn Effect>>,
    accent: [u8; 4],
    theme: Theme,
    effect: EffectWindow,
    _masks: Vec<MaskWindow>,
}

struct CaptureEntry {
    map: CaptureMap,
    created: Instant,
}

struct State {
    started: Instant,
    last_activity: Instant,
    active: Option<Active>,
    timer_ms: u32,
    should_exit: bool,
    captures: VecDeque<CaptureEntry>,
    /// 多步流程里还没播的步骤。
    queue: VecDeque<ShowParams>,
    next_capture: u64,
    hotkey: bool,
    /// 上一次画面是怎么结束的：completed / stopped。
    ended: Option<String>,
    cfg: Config,
}

impl State {
    fn new() -> Self {
        let loaded = config::load(&paths::config_path());
        Self {
            started: Instant::now(),
            last_activity: Instant::now(),
            queue: VecDeque::new(),
            ended: None,
            active: None,
            timer_ms: 0,
            should_exit: false,
            captures: VecDeque::new(),
            next_capture: 1,
            hotkey: false,
            cfg: loaded.config,
        }
    }

    fn arm(&mut self) {
        let want = match &self.active {
            // 入场动画期间用高刷新率，动画结束立刻回到 100ms。
            Some(active) if active.shown_at.elapsed().as_secs_f32() * 1000.0 < ANIM_MS => {
                TICK_ANIM_MS
            }
            // 画面在的时候要能用 30ms 的节奏轮询点击，否则静态画面收不到点击。
            Some(_) => TICK_ACTIVE_MS.min(TICK_CLICK_MS),
            None => TICK_IDLE_MS,
        };
        if self.timer_ms != want {
            message::stop_timer();
            message::start_timer(want);
            self.timer_ms = want;
        }
    }

    fn tick(&mut self) {
        if take_display_change() {
            self.clear_all();
            return;
        }
        if take_dismiss_request() {
            self.finish("stopped");
            return;
        }
        if take_action_request() {
            self.on_action();
            return;
        }

        let pause_cap = Duration::from_secs(self.cfg.pause_cap_secs);
        let mut finished = false;
        if let Some(active) = self.active.as_mut() {
            let now = Instant::now();
            let elapsed = now.saturating_duration_since(active.last_tick);
            active.last_tick = now;
            let opacity = if active.animate {
                (now.saturating_duration_since(active.shown_at).as_secs_f32() * 1000.0 / ANIM_MS)
                    .clamp(0.0, 1.0)
            } else {
                1.0
            };

            // 悬停文案：暂停倒计时。只在有倒计时时才值得轮询。
            if active.total.is_some() {
                let (cx, cy) = cursor_pos();
                let hovering = active
                    .card_screen
                    .is_some_and(|card| card.contains_point(cx, cy));
                // 暂停有上限：超过 60 秒就不再暂停，避免遮罩永远不消失。
                active.paused = hovering && active.paused_total < pause_cap;
                if active.paused {
                    active.paused_total += elapsed;
                } else {
                    active.remaining = active.remaining.saturating_sub(elapsed);
                }
                if active.remaining.is_zero() {
                    finished = true;
                }
            }

            // 动画期间每帧重画；动画结束后只有倒计时进度条需要刷新。
            if !finished && (opacity < 1.0 || active.total.is_some()) {
                repaint(active, opacity);
                // 到 1.0 的这一帧必须画出来，否则画面会停在动画的倒数第二帧上
                // （看着像"卡片有点透"的幽灵残影）。
                if opacity >= 1.0 {
                    active.animate = false;
                }
            }
        }

        if finished {
            self.clear_all();
        }
        if self.active.is_none() && self.last_activity.elapsed() >= IDLE_EXIT {
            self.should_exit = true;
        }
        self.arm();
    }

    fn clear_all(&mut self) {
        if self.active.take().is_some() {
            self.last_activity = Instant::now();
        }
        set_stop_hit(None);
        set_action_hit(None);
        if self.hotkey {
            hotkey::unregister(message::HOTKEY_ID);
            self.hotkey = false;
        }
        self.arm();
    }

    fn find_capture(&self, id: &str) -> Option<&CaptureMap> {
        self.captures
            .iter()
            .find(|entry| entry.map.id.0 == id)
            .map(|entry| &entry.map)
    }

    /// 重新读取配置。只影响之后的 show；正在显示的画面保持原样。
    fn reload(&mut self) -> ReloadResult {
        let loaded = config::load(&paths::config_path());
        let changed = loaded.config != self.cfg;
        self.cfg = loaded.config;

        // 画面正在显示时，热键立刻按新配置重注册
        if self.active.is_some() {
            hotkey::unregister(message::HOTKEY_ID);
            let spec =
                hotkey::parse(&self.cfg.hotkey).unwrap_or_else(|_| hotkey::default_dismiss());
            self.hotkey = hotkey::register(message::HOTKEY_ID, spec);
        }

        ReloadResult {
            reloaded: true,
            changed,
            hotkey: self.cfg.hotkey.clone(),
            note: loaded.note,
        }
    }

    fn captures(&self) -> CapturesResult {
        CapturesResult {
            captures: self
                .captures
                .iter()
                .map(|entry| CaptureSummary {
                    capture: entry.map.id.0.clone(),
                    width: entry.map.image_size.0,
                    height: entry.map.image_size.1,
                    age_ms: entry.created.elapsed().as_millis() as u64,
                })
                .collect(),
        }
    }

    fn status(&self) -> StatusResult {
        StatusResult {
            alive: true,
            version: spotlight_core::VERSION.to_string(),
            uptime_ms: self.started.elapsed().as_millis() as u64,
            hotkey: self.hotkey,
            ended: self.ended.clone(),
            scene: self.active.as_ref().map(|a| SceneSummary {
                key: a.key.clone(),
                targets: a.targets,
                has_text: a.has_text,
                remaining_ms: a.total.map(|_| a.remaining.as_millis() as u64),
            }),
        }
    }

    fn capture(&mut self, params: CaptureParams) -> Result<CaptureInfo, ErrorBody> {
        let monitors = monitors::enumerate_or_primary();
        let index = match params.monitor {
            Some(index) => index as usize,
            None => monitors.iter().position(|m| m.primary).unwrap_or(0),
        };
        let monitor = monitors.get(index).ok_or_else(|| {
            ErrorBody::new(codes::BAD_PARAMS, format!("没有第 {index} 块屏"))
                .with_hint(format!("当前共有 {} 块屏，序号从 0 开始", monitors.len()))
        })?;

        let shot = shot::grab(monitor.bounds).map_err(|e| {
            ErrorBody::new(codes::CAPTURE_FAILED, e).with_hint("确认没有全屏独占应用遮挡")
        })?;
        let max_width = params.max_width.unwrap_or(DEFAULT_CAPTURE_WIDTH);
        let shot = shot::downscale(&shot, max_width).unwrap_or(shot);

        let id = format!("c-{}", self.next_capture);
        self.next_capture += 1;

        let path = paths::captures_dir().join(format!("{id}.png"));
        shot::save_png(&path, shot.width as u32, shot.height as u32, &shot.bgra).map_err(|e| {
            ErrorBody::new(codes::CAPTURE_FAILED, e)
                .with_hint("检查数据目录是否可写，或者换一个 SPOTLIGHT_HOME")
        })?;

        let image_rect = Rect::new(0, 0, shot.width, shot.height);
        let map = CaptureMap {
            id: CaptureId::new(&id),
            image_size: (shot.width as u32, shot.height as u32),
            monitors: monitors.clone(),
            segments: vec![Segment::new(monitor.id, image_rect, monitor.bounds)],
        };
        self.captures.push_back(CaptureEntry {
            map,
            created: Instant::now(),
        });
        while self.captures.len() > MAX_CAPTURES {
            if let Some(old) = self.captures.pop_front() {
                let _ = std::fs::remove_file(
                    paths::captures_dir().join(format!("{}.png", old.map.id.0)),
                );
            }
        }

        Ok(CaptureInfo {
            capture: id,
            image: CaptureImage {
                path: path.display().to_string(),
                width: shot.width as u32,
                height: shot.height as u32,
            },
            monitors: monitors
                .iter()
                .map(|m| MonitorInfo {
                    index: m.id.0,
                    name: m.name.clone(),
                    primary: m.primary,
                    bounds: [m.bounds.x, m.bounds.y, m.bounds.w, m.bounds.h],
                    work: [m.work.x, m.work.y, m.work.w, m.work.h],
                    scale: m.scale,
                })
                .collect(),
            segments: vec![SegmentInfo {
                monitor: monitor.id.0,
                image_rect: [0, 0, shot.width, shot.height],
                screen_rect: [
                    monitor.bounds.x,
                    monitor.bounds.y,
                    monitor.bounds.w,
                    monitor.bounds.h,
                ],
            }],
            virtual_screen: virtual_screen(&monitors),
        })
    }
    /// 多步流程的入口：把 `steps` 展开成队列，先播第一步。
    fn show(&mut self, mut p: ShowParams) -> Result<ShowResult, ErrorBody> {
        self.ended = None;
        let steps = std::mem::take(&mut p.steps);
        if steps.is_empty() {
            self.queue.clear();
            return self.show_single(p);
        }

        // 除 targets/文案之外的参数（坐标空间、超时、效果、key…）各步共用。
        let base = p.clone();
        let mut queue: VecDeque<ShowParams> =
            steps.into_iter().map(|step| to_show(&base, step)).collect();

        let Some(first) = queue.pop_front() else {
            return self.show_single(base);
        };
        self.queue = queue;
        self.show_single(first)
    }

    /// 按钮被按下：下一步 / 完成。
    fn on_action(&mut self) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        match active.action {
            Action::Next => self.advance(),
            Action::Done => self.finish("completed"),
        }
    }

    /// 前进到下一步；没有下一步就按"完成"处理。
    fn advance(&mut self) {
        self.clear_all();
        match self.queue.pop_front() {
            Some(next) => {
                if let Err(err) = self.show_single(next) {
                    eprintln!("下一步失败: {}", err.message);
                    self.finish("stopped");
                }
            }
            None => self.finish("completed"),
        }
    }

    /// 结束当前画面，并记下这次是怎么结束的，供调用方查询。
    fn finish(&mut self, outcome: &str) {
        self.ended = Some(outcome.to_string());
        self.clear_all();
    }
    fn show_single(&mut self, p: ShowParams) -> Result<ShowResult, ErrorBody> {
        // 单个 --rect、多个 --rect、以及 targets 数组都合并到这里。
        let mut requested: Vec<([i32; 4], Option<String>)> = Vec::new();
        if let Some(rect) = p.rect {
            requested.push((rect, None));
        }
        for target in &p.targets {
            requested.push((target.rect, target.step.clone()));
        }
        if requested.is_empty() {
            return Err(
                ErrorBody::new(codes::BAD_PARAMS, "没有目标").with_hint("至少给一个 --rect")
            );
        }

        let mut rects = Vec::with_capacity(requested.len());
        for (raw, _) in &requested {
            rects.push(self.resolve_one(&p, *raw)?);
        }
        if rects.iter().any(|r| r.is_empty()) {
            return Err(
                ErrorBody::new(codes::BAD_PARAMS, "矩形为空").with_hint("w 和 h 必须大于 0")
            );
        }

        let monitors = monitors::enumerate_or_primary();
        for rect in &rects {
            if !monitors.iter().any(|m| m.bounds.contains_rect(*rect)) {
                let areas: Vec<String> = monitors
                    .iter()
                    .map(|m| {
                        format!(
                            "({},{}) {}×{}",
                            m.bounds.x, m.bounds.y, m.bounds.w, m.bounds.h
                        )
                    })
                    .collect();
                return Err(
                    ErrorBody::new(codes::RECT_OUT_OF_BOUNDS, "矩形超出屏幕范围")
                        .with_hint(format!("当前显示器范围: {}", areas.join(" / "))),
                );
            }
        }
        // 卡片放在第一个目标所在那块屏的工作区里。
        let host = monitor_for_rect(&monitors, rects[0])
            .and_then(|index| monitors.get(index))
            .or_else(|| monitors.iter().find(|m| m.primary))
            .ok_or_else(|| internal("找不到显示器"))?;
        let work = host.work;
        let mut scene = Scene {
            key: p.key.clone(),
            targets: rects
                .iter()
                .zip(requested.iter())
                .map(|(rect, (_, step))| {
                    let mut target = Target::new(ScreenRect::from_rect(*rect));
                    target.step = step.clone();
                    target
                })
                .collect(),
            callout: None,
            timeout: match p.timeout {
                None => Timeout::Auto,
                Some(secs) if secs <= 0.0 => Timeout::Sticky,
                Some(secs) => Timeout::Seconds(secs),
            },
        };
        if p.text.is_some() || p.title.is_some() {
            // 只有一个带步骤的目标时，步骤号直接当标题；多目标时交给角标。
            let steps: Vec<&String> = scene
                .targets
                .iter()
                .filter_map(|target| target.step.as_ref())
                .collect();
            let step_title = (steps.len() == 1).then(|| format!("第 {} 步", steps[0]));
            scene.callout = Some(Callout {
                title: p.title.clone().or(step_title),
                text: p.text.clone().unwrap_or_default(),
            });
        }
        scene
            .validate()
            .map_err(|e| ErrorBody::new(codes::BAD_PARAMS, e.to_string()).with_hint(e.hint()))?;

        // 先算几何，再建窗口——任何一步失败都不会留下半截画面。
        // 所有尺寸都以「96 DPI 下的像素」为基准，再按目标所在显示器的缩放比放大。
        let scale = host.scale.clamp(0.5, 3.0);
        let px = |value: i32| ((value as f32) * scale).round() as i32;
        let dip = |value: f32| value * scale;

        let preference = match p.placement.as_deref() {
            None | Some("auto") => Preference::Auto,
            Some("top") => Preference::Side(Placement::Top),
            Some("bottom") => Preference::Side(Placement::Bottom),
            Some("left") => Preference::Side(Placement::Left),
            Some("right") => Preference::Side(Placement::Right),
            Some(other) => {
                return Err(
                    ErrorBody::new(codes::BAD_PARAMS, format!("未知的摆放方位: {other}"))
                        .with_hint("可用: auto / top / bottom / left / right"),
                );
            }
        };

        let style = EffectStyle {
            accent: self.cfg.accent,
            ring_width: self.cfg.ring_width,
            radius: self.cfg.radius,
            scale,
        };
        // 显式传了 --effect 就严格校验；配置里的默认组合保持宽松（前向兼容）。
        if !p.effects.is_empty() {
            let unknown: Vec<&str> = p
                .effects
                .iter()
                .filter(|name| !KNOWN_EFFECTS.contains(&name.as_str()))
                .map(String::as_str)
                .collect();
            if !unknown.is_empty() {
                return Err(ErrorBody::new(
                    codes::BAD_PARAMS,
                    format!("未知效果: {}", unknown.join(", ")),
                )
                .with_hint(format!("可用效果: {}", KNOWN_EFFECTS.join(", "))));
            }
        }

        let effect_names = if p.effects.is_empty() {
            self.cfg.default_effects.clone()
        } else {
            p.effects.clone()
        };
        let effects = build_effects(&effect_names, &style);
        let margin = effects
            .iter()
            .map(|e| e.margin())
            .fold(0.0_f32, f32::max)
            .ceil() as i32
            + 2;

        let font_px = px(self.cfg.font_px).max(8);
        let title_px = px(self.cfg.font_px - 2).max(8);
        let text_width = px(self.cfg.max_text_width as i32).max(80) as u32;
        let pad = px(CARD_PADDING).max(4);
        let close_mark_size = px(CLOSE_MARK).max(8);
        let close_margin = px(CLOSE_MARGIN).max(4);
        let close_hit_size = px(CLOSE_HIT).max(16);

        // 每个带步骤的目标做一个小药丸，多目标时能看出先后。
        let mut badge_data: Vec<(Rect, TextBitmap)> = Vec::new();
        if scene.targets.len() > 1 {
            for target in &scene.targets {
                let Some(step) = &target.step else { continue };
                let Some(bitmap) =
                    render_text(step, px(64).max(24) as u32, px(self.cfg.font_px - 3).max(8))
                else {
                    continue;
                };
                let pill_w = bitmap.width as i32 + px(12);
                let pill_h = bitmap.height as i32 + px(6);
                let target_rect = target.rect.rect();
                let area = monitor_for_rect(&monitors, target_rect)
                    .and_then(|index| monitors.get(index))
                    .map(|m| m.bounds)
                    .unwrap_or(host.bounds);
                let x = (target_rect.x - px(6)).clamp(area.x, (area.right() - pill_w).max(area.x));
                let mut y = target_rect.y - pill_h - px(6);
                if y < area.y {
                    y = target_rect.y + px(6);
                }
                let y = y.clamp(area.y, (area.bottom() - pill_h).max(area.y));
                badge_data.push((Rect::new(x, y, pill_w, pill_h), bitmap));
            }
        }

        let title_bitmap = scene
            .callout
            .as_ref()
            .and_then(|c| c.title.as_ref())
            .and_then(|title| render_text(title, text_width, title_px));
        let text_bitmap = scene
            .callout
            .as_ref()
            .filter(|c| !c.text.trim().is_empty())
            .and_then(|c| render_text(&c.text, text_width, font_px));

        // 快捷键提示：卡片底部一个小键帽。
        let key_pad_x = px(8);
        let key_pad_y = px(4);
        let key_gap = px(6);
        let shortcut_bitmap = p
            .shortcut
            .as_ref()
            .filter(|keys| !keys.trim().is_empty())
            .and_then(|keys| {
                render_text(
                    keys,
                    px(180).max(40) as u32,
                    px(self.cfg.font_px - 4).max(8),
                )
            });
        let key_block = shortcut_bitmap
            .as_ref()
            .map(|tb| tb.height as i32 + key_pad_y * 2 + key_gap)
            .unwrap_or(0);

        // 卡片底部一个动作按钮：还有下一步就是「下一步」，否则是「完成」。
        // 两者只会存在一个——这是"什么时候该收尾"的唯一提示。
        let action = if self.queue.is_empty() {
            Action::Done
        } else {
            Action::Next
        };
        let button_label = match action {
            Action::Next => "下一步",
            Action::Done => "完成",
        };
        let button_bitmap = render_text(
            button_label,
            px(120).max(40) as u32,
            px(self.cfg.font_px).max(8),
        );
        let button_pad_x = px(16);
        let button_pad_y = px(5);
        let button_gap = px(14);
        // 按钮宽度有下限：只有两个字的「完成」不该缩成一小团，
        // 和「下一步」放在一起时也该是同一个体量。
        let button_min_w = px(72);
        let button_w = button_bitmap
            .as_ref()
            .map(|tb| (tb.width as i32 + button_pad_x * 2).max(button_min_w));
        let button_block = button_bitmap
            .as_ref()
            .map(|tb| tb.height as i32 + button_pad_y * 2 + button_gap)
            .unwrap_or(0);

        // 卡片相对"所有目标的并集"摆放，多目标时不会只贴住其中一个。
        let target_union = rects.iter().fold(Rect::ZERO, |acc, r| acc.union(*r));
        let title_block = title_bitmap
            .as_ref()
            .map(|tb| tb.height as i32 + px(TITLE_GAP))
            .unwrap_or(0);
        let content_w = title_bitmap
            .as_ref()
            .map(|tb| tb.width as i32)
            .into_iter()
            .chain(text_bitmap.as_ref().map(|tb| tb.width as i32))
            .chain(button_w)
            .max()
            .unwrap_or(0);
        let body_h = text_bitmap.as_ref().map(|tb| tb.height as i32).unwrap_or(0);
        let content_h = title_block + body_h + key_block + button_block;

        let card_placed = (content_w > 0).then(|| {
            place_preferring(
                target_union,
                Size::new(
                    content_w + pad + px(CARD_PADDING_RIGHT),
                    content_h + pad * 2,
                ),
                work,
                px(CARD_GAP),
                preference,
            )
        });
        let card_rect = card_placed.map(|placed| placed.rect);

        let debug = ShowDebug {
            monitor: host.name.clone(),
            scale: host.scale,
            placement: card_placed
                .map(|placed| format!("{:?}", placed.placement).to_lowercase())
                .unwrap_or_else(|| "none".to_string()),
            card: card_rect.map(|card| [card.x, card.y, card.w, card.h]),
            font_px,
        };

        let mut union = rects
            .iter()
            .fold(Rect::ZERO, |acc, r| acc.union(r.inflate(margin)));
        if let Some(card) = card_rect {
            union = union.union(card);
        }
        for (rect, _) in &badge_data {
            union = union.union(*rect);
        }

        let offset = |r: Rect| r.translated(-union.x, -union.y);

        let close_mark = card_rect.map(|card| {
            Rect::new(
                card.right() - close_margin - close_mark_size,
                card.y + close_margin,
                close_mark_size,
                close_mark_size,
            )
        });
        let close_hit_screen = close_mark.map(|mark| {
            Rect::new(
                mark.x + mark.w / 2 - close_hit_size / 2,
                mark.y + mark.h / 2 - close_hit_size / 2,
                close_hit_size,
                close_hit_size,
            )
        });

        let button_rects = card_rect.and_then(|card| {
            button_bitmap.as_ref().map(|tb| {
                // 按文字宽度自适应，左边和标题、正文对齐；不再铺满整行。
                let width = (tb.width as i32 + button_pad_x * 2).max(button_min_w);
                let pill = Rect::new(
                    card.x + pad,
                    card.y + pad + title_block + body_h + key_block,
                    width,
                    tb.height as i32 + button_pad_y * 2,
                );
                let text = Rect::new(
                    pill.x + (width - tb.width as i32) / 2,
                    pill.y + button_pad_y,
                    tb.width as i32,
                    tb.height as i32,
                );
                (offset(pill), offset(text))
            })
        });
        let action_pill = button_rects.map(|(pill, _)| pill);

        let geom = Geom {
            union,
            card_radius: dip(CARD_RADIUS),
            targets: rects.iter().map(|r| offset(*r)).collect(),
            card: card_rect.map(offset),
            close: close_mark.map(offset),
            badges: badge_data
                .into_iter()
                .map(|(rect, bitmap)| (offset(rect), bitmap))
                .collect(),
            keycap: card_rect.and_then(|card| {
                shortcut_bitmap.as_ref().map(|tb| {
                    offset(Rect::new(
                        card.x + pad,
                        card.y + pad + title_block + body_h + key_gap,
                        tb.width as i32 + key_pad_x * 2,
                        tb.height as i32 + key_pad_y * 2,
                    ))
                })
            }),
            keycap_text: card_rect.and_then(|card| {
                shortcut_bitmap.as_ref().map(|tb| {
                    offset(Rect::new(
                        card.x + pad + key_pad_x,
                        card.y + pad + title_block + body_h + key_gap + key_pad_y,
                        tb.width as i32,
                        tb.height as i32,
                    ))
                })
            }),
            button: button_rects,

            title_at: card_rect.and_then(|card| {
                title_bitmap.as_ref().map(|_| {
                    (
                        (card.x + pad - union.x) as f32,
                        (card.y + pad - union.y) as f32,
                    )
                })
            }),
            text_at: card_rect.and_then(|card| {
                text_bitmap.as_ref().map(|_| {
                    (
                        (card.x + pad - union.x) as f32,
                        (card.y + pad + title_block - union.y) as f32,
                    )
                })
            }),
        };
        let same_key = match (&p.key, self.active.as_ref()) {
            (Some(new_key), Some(active)) => active.key.as_deref() == Some(new_key.as_str()),
            _ => false,
        };

        // 文字被裁掉要告诉调用方，别让它以为用户看到了完整内容。
        let mut warnings = Vec::new();
        if title_bitmap.as_ref().is_some_and(|tb| tb.clipped) {
            warnings.push("标题过长，已截断".to_string());
        }
        if text_bitmap.as_ref().is_some_and(|tb| tb.clipped) {
            warnings.push("正文过长，已截断；建议拆成多步".to_string());
        }
        if shortcut_bitmap.as_ref().is_some_and(|tb| tb.clipped) {
            warnings.push("快捷键提示过长，已截断".to_string());
        }

        let timeout = resolve(scene.timeout, scene.text());

        // 只校验：把换算后的屏幕矩形和预计时长报回去，不建任何窗口。
        if p.dry_run {
            return Ok(ShowResult {
                scene: scene.key.clone().unwrap_or_else(|| "s-1".into()),
                timeout_ms: timeout.map(|d| d.as_millis() as u64),
                dry_run: true,
                targets: rects.iter().map(|r| [r.x, r.y, r.w, r.h]).collect(),
                effects: effect_names.clone(),
                warnings,
                debug: Some(debug),
            });
        }

        self.clear_all();

        // 洞只比目标大一点点：圆形的圈画应当落在压暗的背景上，而不是落在一块亮矩形里。
        let hole_pad = px(PAD).max(1);
        let holes: Vec<Rect> = rects.iter().map(|r| r.inflate(hole_pad)).collect();
        let dim_alpha = (self.cfg.dim * 255.0).round().clamp(0.0, 255.0) as u8;
        let mut masks = Vec::new();
        for monitor in &monitors {
            if !rects.iter().any(|r| r.intersect(monitor.bounds).is_some()) {
                continue;
            }
            let plan = mask::plan(monitor.bounds, &holes);
            if plan.fallback_fullscreen {
                return Err(
                    ErrorBody::new(codes::BAD_PARAMS, "场景太复杂：遮罩要拆成太多窗口")
                        .with_hint("一次 show 的目标控制在 8 个以内，或者拆成几次"),
                );
            }
            for r in &plan.rects {
                let window = MaskWindow::new(*r, dim_alpha).map_err(internal)?;
                masks.push(window);
            }
        }

        // 效果窗口必须**最后**创建：同一进程里后创建的 topmost 窗口在上面。
        // 反过来，遮罩会盖住卡片——看起来就像"气泡在遮罩后面"。

        let active = Active {
            key: scene.key.clone(),
            has_text: scene.callout.is_some(),
            targets: scene.targets.len(),
            total: timeout,
            remaining: timeout.unwrap_or_default(),
            last_tick: Instant::now(),
            shown_at: Instant::now(),
            paused: false,
            paused_total: Duration::ZERO,
            animate: !same_key,
            card_screen: card_rect,
            geom,
            title: title_bitmap,
            text: text_bitmap,
            keycap: shortcut_bitmap,
            button: button_bitmap,
            action,
            holes: holes.iter().map(|r| offset(*r)).collect(),
            hole_radius: dip(self.cfg.radius) + hole_pad as f32,
            dim: [0, 0, 0, dim_alpha],
            effects,
            accent: self.cfg.accent,
            theme: self.cfg.theme,
            effect: EffectWindow::new(union).map_err(internal)?,
            _masks: masks,
        };

        let initial_opacity = if same_key { 1.0 } else { 0.0 };
        let Some(bgra) = render_active(&active, initial_opacity) else {
            return Err(internal("渲染失败"));
        };

        if !active
            .effect
            .set_pixels(union.x, union.y, union.w, union.h, &bgra)
        {
            return Err(internal("提交位图失败"));
        }
        // 调试构建允许关掉截图排除，方便用探针看到真实画面。
        #[cfg(debug_assertions)]
        let excluded = std::env::var_os("SPOTLIGHT_DEBUG_NO_EXCLUDE").is_none();
        #[cfg(not(debug_assertions))]
        let excluded = true;
        active.effect.set_excluded(excluded);

        set_stop_hit(close_hit_screen.map(offset));
        set_action_hit(action_pill);
        let _ = take_dismiss_request();

        self.active = Some(active);
        self.last_activity = Instant::now();
        self.arm();
        // 只在有画面时占用热键，画面消失立刻归还。
        let spec = hotkey::parse(&self.cfg.hotkey).unwrap_or_else(|_| hotkey::default_dismiss());
        self.hotkey = hotkey::register(message::HOTKEY_ID, spec);

        Ok(ShowResult {
            scene: scene.key.clone().unwrap_or_else(|| "s-1".into()),
            timeout_ms: timeout.map(|d| d.as_millis() as u64),
            dry_run: false,
            targets: rects.iter().map(|r| [r.x, r.y, r.w, r.h]).collect(),
            effects: effect_names,
            warnings,
            debug: Some(debug),
        })
    }

    /// 把调用方给的矩形换算成屏幕物理像素。D5a：必须显式声明坐标空间。
    fn resolve_one(&self, p: &ShowParams, rect: [i32; 4]) -> Result<Rect, ErrorBody> {
        if let Some(capture_id) = &p.capture {
            let map = self.find_capture(capture_id).ok_or_else(|| {
                ErrorBody::new(codes::UNKNOWN_CAPTURE, format!("找不到截图 {capture_id}"))
                    .with_hint("截图可能已过期，重新运行 spotlight capture")
            })?;
            let mapped = map
                .to_screen(ImageRect::new(rect[0], rect[1], rect[2], rect[3]))
                .map_err(|e| {
                    ErrorBody::new(codes::BAD_PARAMS, e.to_string())
                        .with_hint("坐标必须落在截图范围内")
                })?;
            return Ok(mapped.rect.rect());
        }

        if p.space.as_deref() == Some("screen") {
            return Ok(Rect::new(rect[0], rect[1], rect[2], rect[3]));
        }

        Err(
            ErrorBody::new(codes::SPACE_UNSPECIFIED, "坐标空间未指定").with_hint(
                "先运行 spotlight capture 拿到截图，再用 --capture <id> 传坐标；\
                 或者显式加 --space screen 表示坐标已经是屏幕物理像素",
            ),
        )
    }
}

/// 内部错误统一带上同一条排查建议。
fn internal(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(codes::INTERNAL, message)
        .with_hint("运行 spotlight doctor 排查；必要时结束 spotlight.exe 再重试")
}

/// 上一次守护进程留下的截图文件已经无法被引用（id 只活在内存里），启动时清掉。
fn cleanup_captures(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("png") {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// 把一步展开成一次完整的 show 参数，除目标与文案外都沿用外层。
fn to_show(base: &ShowParams, step: StepSpec) -> ShowParams {
    ShowParams {
        rect: None,
        targets: step.targets,
        capture: base.capture.clone(),
        space: base.space.clone(),
        text: step.text,
        title: step.title,
        step: None,
        shortcut: step.shortcut,
        placement: base.placement.clone(),
        dry_run: base.dry_run,
        effects: base.effects.clone(),
        timeout: base.timeout,
        key: base.key.clone(),
        steps: Vec::new(),
    }
}

fn virtual_screen(monitors: &[spotlight_core::monitor::Monitor]) -> RectInfo {
    let mut union = Rect::ZERO;
    for monitor in monitors {
        union = union.union(monitor.bounds);
    }
    RectInfo {
        x: union.x,
        y: union.y,
        width: union.w,
        height: union.h,
    }
}

fn render_active(active: &Active, opacity: f32) -> Option<Vec<u8>> {
    let progress = match active.total {
        Some(total) if total.as_secs_f32() > 0.0 => Some(Progress {
            value: (active.remaining.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0),
            color: active.accent,
        }),
        _ => None,
    };

    let hole_views: Vec<spotlight_render::Hole> = active
        .holes
        .iter()
        .map(|rect| spotlight_render::Hole {
            rect: RectF::from_rect(*rect),
            radius: active.hole_radius,
        })
        .collect();

    let badges: Vec<Badge> = active
        .geom
        .badges
        .iter()
        .map(|(rect, bitmap)| Badge {
            rect: RectF::from_rect(*rect),
            radius: rect.h as f32 / 2.0,
            bg: active.accent,
            text: TextLayer {
                rect: RectF::new(
                    rect.x as f32 + 6.0,
                    rect.y as f32 + 3.0,
                    bitmap.width as f32,
                    bitmap.height as f32,
                ),
                mask: &bitmap.alpha,
                mask_width: bitmap.width,
                mask_height: bitmap.height,
                color: [255, 255, 255, 255],
            },
        })
        .collect();

    let ctx = Ctx {
        card: active.geom.card.map(|c| Card {
            rect: RectF::from_rect(c),
            radius: active.geom.card_radius,
            bg: active.theme.card_bg(),
        }),
        keycap: match (&active.keycap, active.geom.keycap, active.geom.keycap_text) {
            (Some(tb), Some(pill), Some(text_rect)) => Some(Keycap {
                rect: RectF::from_rect(pill),
                radius: (pill.h as f32) * 0.3,
                bg: active.theme.keycap_bg(),
                border: active.theme.keycap_border(),
                text: TextLayer {
                    rect: RectF::from_rect(text_rect),
                    mask: &tb.alpha,
                    mask_width: tb.width,
                    mask_height: tb.height,
                    color: active.theme.text(),
                },
            }),
            _ => None,
        },
        button: match (&active.button, active.geom.button) {
            (Some(tb), Some((pill, text))) => Some(Button {
                rect: RectF::from_rect(pill),
                radius: (pill.h as f32) / 2.0,
                bg: active.theme.button_bg(active.accent),
                text: TextLayer {
                    rect: RectF::from_rect(text),
                    mask: &tb.alpha,
                    mask_width: tb.width,
                    mask_height: tb.height,
                    color: active.theme.button_text(),
                },
            }),
            _ => None,
        },
        holes: &hole_views,
        dim: active.dim,
        badges: &badges,
        title: match (&active.title, active.geom.title_at) {
            (Some(tb), Some((x, y))) => Some(TextLayer {
                rect: RectF::new(x, y, tb.width as f32, tb.height as f32),
                mask: &tb.alpha,
                mask_width: tb.width,
                mask_height: tb.height,
                color: active.theme.title(),
            }),
            _ => None,
        },
        text: match (&active.text, active.geom.text_at) {
            (Some(tb), Some((x, y))) => Some(TextLayer {
                rect: RectF::new(x, y, tb.width as f32, tb.height as f32),
                mask: &tb.alpha,
                mask_width: tb.width,
                mask_height: tb.height,
                color: active.theme.text(),
            }),
            _ => None,
        },
        progress,
        stop: active.geom.close.map(|c| StopMark {
            rect: RectF::from_rect(c),
            color: active.theme.stop(),
        }),
    };

    let targets: Vec<RectF> = active
        .geom
        .targets
        .iter()
        .map(|r| RectF::from_rect(*r))
        .collect();

    // 先把要画的东西算出来，最后统一给高亮区域补一层 alpha=1 的"实体"。
    // 注意必须在这里做：动画期间每帧重绘，在外面补会被下一帧覆盖掉。
    let mut frame = render_frame(
        active.geom.union.w as u32,
        active.geom.union.h as u32,
        &targets,
        &active.effects,
        &ctx,
        opacity,
    )?;

    // 分层窗口按像素 alpha 做命中测试：透明的地方点击会直接穿过去。
    // 给高亮区域一个近乎不可见的 alpha，点击才会落到我们身上，再转发给下面的应用。
    let width = active.geom.union.w;
    let height = active.geom.union.h;
    for target in &active.geom.targets {
        for y in target.y.max(0)..target.bottom().min(height) {
            for x in target.x.max(0)..target.right().min(width) {
                let index = ((y * width + x) * 4 + 3) as usize;
                if index < frame.len() && frame[index] == 0 {
                    frame[index] = 1;
                }
            }
        }
    }

    Some(frame)
}

fn repaint(active: &Active, opacity: f32) {
    if let Some(bgra) = render_active(active, opacity) {
        let g = active.geom.union;
        active.effect.set_pixels(g.x, g.y, g.w, g.h, &bgra);
    }
}

fn handle(state: &mut State, req: Request) -> Response {
    let id = req.id;
    if let Err(err) = req.check_version() {
        return Response::err(id, err);
    }

    match req.method {
        Method::Ping => Response::ok(id, json!({"pong": true})),
        Method::Status => {
            let status = state.status();
            Response::ok(id, serde_json::to_value(status).unwrap_or(json!(null)))
        }
        Method::Capture => {
            let params: CaptureParams = match req.parse_params() {
                Ok(p) => p,
                Err(err) => return Response::err(id, err),
            };
            match state.capture(params) {
                Ok(info) => Response::ok(id, serde_json::to_value(info).unwrap_or(json!(null))),
                Err(err) => Response::err(id, err),
            }
        }
        Method::Reload => {
            let result = state.reload();
            Response::ok(id, serde_json::to_value(result).unwrap_or(json!(null)))
        }
        Method::Captures => {
            let result = state.captures();
            Response::ok(id, serde_json::to_value(result).unwrap_or(json!(null)))
        }
        Method::Clear => {
            let params: ClearParams = match req.parse_params() {
                Ok(params) => params,
                Err(err) => return Response::err(id, err),
            };
            if let Some(key) = params.key {
                let matches = state
                    .active
                    .as_ref()
                    .and_then(|active| active.key.as_deref())
                    == Some(key.as_str());
                if !matches {
                    return Response::err(
                        id,
                        ErrorBody::new(codes::NO_SUCH_KEY, format!("没有 key 为 {key} 的画面"))
                            .with_hint(
                                "用 spotlight status 看当前画面的 key；不带 --key 可以清掉任何画面",
                            ),
                    );
                }
            }
            state.clear_all();
            Response::ok(id, json!({"cleared": true}))
        }
        Method::Show => {
            let params: ShowParams = match req.parse_params() {
                Ok(p) => p,
                Err(err) => return Response::err(id, err),
            };
            match state.show(params) {
                Ok(result) => Response::ok(id, serde_json::to_value(result).unwrap_or(json!(null))),
                Err(err) => Response::err(id, err),
            }
        }
    }
}

fn send(conn: &mut Connection, resp: &Response) {
    if let Ok(line) = serde_json::to_string(resp) {
        let _ = conn.send(&line);
    }
}

fn serve(conn: &mut Connection, sender: &message::Sender<Job>) {
    loop {
        match conn.recv() {
            Ok(Some(line)) => {
                let req: Request = match serde_json::from_str(&line) {
                    Ok(req) => req,
                    Err(err) => {
                        send(
                            conn,
                            &Response::err(
                                0,
                                ErrorBody::new(codes::BAD_JSON, err.to_string())
                                    .with_hint("请求必须是单行 JSON，字段参考 spotlight --help"),
                            ),
                        );
                        continue;
                    }
                };
                let id = req.id;
                let (tx, rx) = mpsc::sync_channel(1);
                sender.send(Job { req, reply: tx });
                match rx.recv_timeout(REQUEST_TIMEOUT) {
                    Ok(resp) => send(conn, &resp),
                    Err(_) => send(
                        conn,
                        &Response::err(
                            id,
                            ErrorBody::new(codes::TIMEOUT, "守护进程处理超时")
                                .with_hint("重试一次；如果持续超时，结束 spotlight.exe 后再试"),
                        ),
                    ),
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
}

/// 守护进程入口。
pub fn run() -> i32 {
    enable_dpi_awareness();
    let _guard = match SingleInstance::acquire(&format!(
        "Local\\spotlight-daemon-{:08x}",
        ipc::binary_tag()
    )) {
        Ok(guard) => guard,
        Err(_) => return 0, // 已经有一个在跑
    };

    cleanup_captures(&paths::captures_dir());

    let bus = MessageBus::<Job>::new();
    let sender = bus.sender();
    let server = Server::new(&ipc::pipe_name());

    std::thread::spawn(move || {
        loop {
            let mut conn = match server.accept() {
                Ok(conn) => conn,
                Err(_) => break,
            };
            // 每个连接一个线程：某个客户端卡住不发数据，不能把整个守护进程堵死。
            let sender = sender.clone();
            std::thread::spawn(move || serve(&mut conn, &sender));
        }
    });

    let mut state = State::new();
    state.arm();

    while let Some(event) = bus.next_event() {
        match event {
            Event::Command(job) => {
                let resp = handle(&mut state, job.req);
                let _ = job.reply.send(resp);
            }
            Event::Timer => state.tick(),
            // 多步流程里热键是"下一步"，没有下一步才关掉。
            Event::Hotkey => state.on_action(),
        }
        if state.should_exit {
            break;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_removes_only_png_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("test-capture-cleanup");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("c-1.png"), b"x").unwrap();
        std::fs::write(dir.join("keep.txt"), b"x").unwrap();

        cleanup_captures(&dir);

        assert!(!dir.join("c-1.png").exists(), "png 应该被清掉");
        assert!(dir.join("keep.txt").exists(), "不该碰别的文件");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_on_missing_dir_is_a_noop() {
        cleanup_captures(std::path::Path::new("D:/definitely/not/here"));
    }
}
