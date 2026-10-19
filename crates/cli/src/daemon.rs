//! 守护进程：持有所有 overlay 窗口，串行处理 IPC 请求。
//!
//! 所有窗口操作都在主线程；IPC 线程只负责收包和回包。

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::json;
use spotlight_core::geom::{ImageRect, Rect, RectF, ScreenRect};
use spotlight_core::layout::{Size, place};
use spotlight_core::mapping::{CaptureId, CaptureMap, Segment};
use spotlight_core::mask::subtract;
use spotlight_core::monitor::MonitorId;
use spotlight_core::protocol::{
    CaptureImage, CaptureInfo, CaptureParams, ClearParams, ErrorBody, Method, MonitorInfo,
    RectInfo, Request, Response, SceneSummary, SegmentInfo, ShowParams, ShowResult, StatusResult,
    codes,
};
use spotlight_core::scene::{Callout, Scene, Target};
use spotlight_core::timing::{Timeout, resolve};
use spotlight_platform::win::effect::EffectWindow;
use spotlight_platform::win::ipc::{self, Connection, Server};
use spotlight_platform::win::message::{self, Event, MessageBus};
use spotlight_platform::win::process::SingleInstance;
use spotlight_platform::win::shot;
use spotlight_platform::win::text::{TextBitmap, render_text};
use spotlight_platform::win::{
    MaskWindow, cursor_pos, enable_dpi_awareness, primary_scale, screen_size, set_close_hit,
    take_dismiss_request,
};
use spotlight_render::{
    Card, CloseMark, Ctx, Effect, Progress, TextLayer, build_effects, render_frame,
};

const DIM_ALPHA: u8 = 140;
/// 洞相对目标的 padding。
const PAD: i32 = 6;
const ACCENT: [u8; 4] = [255, 59, 48, 255];
const CARD_BG: [u8; 4] = [27, 27, 31, 240];
const CARD_RADIUS: f32 = 10.0;
const CARD_PADDING: i32 = 12;
/// 右侧多留一点，避免文字顶到关闭叉。
const CARD_PADDING_RIGHT: i32 = 26;
const CARD_GAP: i32 = 12;
const FONT_PX: i32 = 14;
const MAX_TEXT_WIDTH: u32 = 336;

const CLOSE_MARK: i32 = 12;
const CLOSE_HIT: i32 = 28;
const CLOSE_MARGIN: i32 = 8;

const TICK_ACTIVE_MS: u32 = 100;
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
#[derive(Clone, Debug)]
struct Geom {
    union: Rect,
    targets: Vec<Rect>,
    card: Option<Rect>,
    close: Option<Rect>,
    text_at: Option<(f32, f32)>,
}

struct Active {
    key: Option<String>,
    has_text: bool,
    total: Option<Duration>,
    remaining: Duration,
    last_tick: Instant,
    shown_at: Instant,
    paused: bool,
    card_screen: Option<Rect>,
    geom: Geom,
    text: Option<TextBitmap>,
    effects: Vec<Box<dyn Effect>>,
    effect: EffectWindow,
    _masks: Vec<MaskWindow>,
}

struct State {
    started: Instant,
    last_activity: Instant,
    active: Option<Active>,
    timer_ms: u32,
    should_exit: bool,
    captures: VecDeque<(String, CaptureMap)>,
    next_capture: u64,
}

impl State {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            last_activity: Instant::now(),
            active: None,
            timer_ms: 0,
            should_exit: false,
            captures: VecDeque::new(),
            next_capture: 1,
        }
    }

    fn arm(&mut self) {
        let want = match &self.active {
            // 入场动画期间用高刷新率，动画结束立刻回到 100ms。
            Some(active) if active.shown_at.elapsed().as_secs_f32() * 1000.0 < ANIM_MS => {
                TICK_ANIM_MS
            }
            Some(_) => TICK_ACTIVE_MS,
            None => TICK_IDLE_MS,
        };
        if self.timer_ms != want {
            message::stop_timer();
            message::start_timer(want);
            self.timer_ms = want;
        }
    }

    fn tick(&mut self) {
        if take_dismiss_request() {
            self.clear_all();
            return;
        }

        let mut finished = false;
        if let Some(active) = self.active.as_mut() {
            let now = Instant::now();
            let elapsed = now.saturating_duration_since(active.last_tick);
            active.last_tick = now;
            let opacity = (now.saturating_duration_since(active.shown_at).as_secs_f32() * 1000.0
                / ANIM_MS)
                .clamp(0.0, 1.0);

            // 悬停文案：暂停倒计时。只在有倒计时时才值得轮询。
            if active.total.is_some() {
                let (cx, cy) = cursor_pos();
                active.paused = active
                    .card_screen
                    .is_some_and(|card| card.contains_point(cx, cy));
                if !active.paused {
                    active.remaining = active.remaining.saturating_sub(elapsed);
                }
                if active.remaining.is_zero() {
                    finished = true;
                }
            }

            // 动画期间每帧重画；动画结束后只有倒计时进度条需要刷新。
            if !finished && (opacity < 1.0 || active.total.is_some()) {
                repaint(active, opacity);
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
        set_close_hit(None);
        self.arm();
    }

    fn find_capture(&self, id: &str) -> Option<&CaptureMap> {
        self.captures
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, map)| map)
    }

    fn status(&self) -> StatusResult {
        StatusResult {
            alive: true,
            version: spotlight_core::VERSION.to_string(),
            uptime_ms: self.started.elapsed().as_millis() as u64,
            scene: self.active.as_ref().map(|a| SceneSummary {
                key: a.key.clone(),
                targets: 1,
                has_text: a.has_text,
                remaining_ms: a.total.map(|_| a.remaining.as_millis() as u64),
            }),
        }
    }

    fn capture(&mut self, params: CaptureParams) -> Result<CaptureInfo, ErrorBody> {
        let (screen_w, screen_h) = screen_size();
        let bounds = Rect::new(0, 0, screen_w, screen_h);

        let shot = shot::grab(bounds).map_err(|e| {
            ErrorBody::new(codes::CAPTURE_FAILED, e).with_hint("确认没有全屏独占应用遮挡")
        })?;
        let max_width = params.max_width.unwrap_or(DEFAULT_CAPTURE_WIDTH);
        let shot = shot::downscale(&shot, max_width).unwrap_or(shot);

        let id = format!("c-{}", self.next_capture);
        self.next_capture += 1;

        let path = capture_dir().join(format!("{id}.png"));
        shot::save_png(&path, shot.width as u32, shot.height as u32, &shot.bgra)
            .map_err(|e| ErrorBody::new(codes::CAPTURE_FAILED, e))?;

        let image_rect = Rect::new(0, 0, shot.width, shot.height);
        let map = CaptureMap {
            id: CaptureId::new(&id),
            image_size: (shot.width as u32, shot.height as u32),
            monitors: vec![spotlight_core::monitor::Monitor::new(
                MonitorId(0),
                "primary",
                bounds,
                bounds,
                1.0,
                true,
            )],
            segments: vec![Segment::new(MonitorId(0), image_rect, bounds)],
        };
        self.captures.push_back((id.clone(), map));
        while self.captures.len() > MAX_CAPTURES {
            if let Some((old, _)) = self.captures.pop_front() {
                let _ = std::fs::remove_file(capture_dir().join(format!("{old}.png")));
            }
        }

        Ok(CaptureInfo {
            capture: id,
            image: CaptureImage {
                path: path.display().to_string(),
                width: shot.width as u32,
                height: shot.height as u32,
            },
            monitors: vec![MonitorInfo {
                index: 0,
                name: "primary".into(),
                primary: true,
                bounds: [bounds.x, bounds.y, bounds.w, bounds.h],
                work: [bounds.x, bounds.y, bounds.w, bounds.h],
                scale: primary_scale(),
            }],
            segments: vec![SegmentInfo {
                monitor: 0,
                image_rect: [0, 0, shot.width, shot.height],
                screen_rect: [bounds.x, bounds.y, bounds.w, bounds.h],
            }],
            virtual_screen: RectInfo {
                x: 0,
                y: 0,
                width: screen_w,
                height: screen_h,
            },
        })
    }

    fn show(&mut self, p: ShowParams) -> Result<ShowResult, ErrorBody> {
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

        let (screen_w, screen_h) = screen_size();
        let bounds = Rect::new(0, 0, screen_w, screen_h);
        for rect in &rects {
            if !bounds.contains_rect(*rect) {
                return Err(
                    ErrorBody::new(codes::RECT_OUT_OF_BOUNDS, "矩形超出屏幕范围").with_hint(
                        format!("当前屏幕是 {screen_w}×{screen_h}，请重新截图后按图像像素给坐标"),
                    ),
                );
            }
        }

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
        if let Some(text) = p.text.clone() {
            scene.callout = Some(Callout {
                title: p.title.clone(),
                text,
            });
        }
        scene
            .validate()
            .map_err(|e| ErrorBody::new(codes::BAD_PARAMS, e.to_string()))?;

        // 先算几何，再建窗口——任何一步失败都不会留下半截画面。
        let effects = build_effects(&p.effects);
        let margin = effects
            .iter()
            .map(|e| e.margin())
            .fold(0.0_f32, f32::max)
            .ceil() as i32
            + 2;

        let text_bitmap = scene
            .callout
            .as_ref()
            .and_then(|c| render_text(&c.text, MAX_TEXT_WIDTH, FONT_PX));

        // 卡片相对"所有目标的并集"摆放，多目标时不会只贴住其中一个。
        let target_union = rects.iter().fold(Rect::ZERO, |acc, r| acc.union(*r));
        let card_rect = text_bitmap.as_ref().map(|tb| {
            place(
                target_union,
                Size::new(
                    tb.width as i32 + CARD_PADDING + CARD_PADDING_RIGHT,
                    tb.height as i32 + CARD_PADDING * 2,
                ),
                bounds,
                CARD_GAP,
            )
            .rect
        });

        let mut union = rects
            .iter()
            .fold(Rect::ZERO, |acc, r| acc.union(r.inflate(margin)));
        if let Some(card) = card_rect {
            union = union.union(card);
        }

        let offset = |r: Rect| r.translated(-union.x, -union.y);

        let close_mark = card_rect.map(|card| {
            Rect::new(
                card.right() - CLOSE_MARGIN - CLOSE_MARK,
                card.y + CLOSE_MARGIN,
                CLOSE_MARK,
                CLOSE_MARK,
            )
        });
        let close_hit_screen = close_mark.map(|mark| {
            Rect::new(
                mark.x + mark.w / 2 - CLOSE_HIT / 2,
                mark.y + mark.h / 2 - CLOSE_HIT / 2,
                CLOSE_HIT,
                CLOSE_HIT,
            )
        });

        let geom = Geom {
            union,
            targets: rects.iter().map(|r| offset(*r)).collect(),
            card: card_rect.map(offset),
            close: close_mark.map(offset),
            text_at: card_rect.map(|card| {
                (
                    (card.x + CARD_PADDING - union.x) as f32,
                    (card.y + CARD_PADDING - union.y) as f32,
                )
            }),
        };

        let timeout = resolve(scene.timeout, scene.text());

        let mut active = Active {
            key: scene.key.clone(),
            has_text: scene.callout.is_some(),
            total: timeout,
            remaining: timeout.unwrap_or_default(),
            last_tick: Instant::now(),
            shown_at: Instant::now(),
            paused: false,
            card_screen: card_rect,
            geom,
            text: text_bitmap,
            effects,
            effect: EffectWindow::new(union).map_err(|e| ErrorBody::new(codes::INTERNAL, e))?,
            _masks: Vec::new(),
        };

        let Some(bgra) = render_active(&active, 0.0) else {
            return Err(ErrorBody::new(codes::INTERNAL, "渲染失败"));
        };

        self.clear_all();

        // 洞只比目标大一点点：圆形的圈画应当落在压暗的背景上，而不是落在一块亮矩形里。
        let hole_pad = PAD;
        let holes: Vec<Rect> = rects.iter().map(|r| r.inflate(hole_pad)).collect();
        let mut masks = Vec::new();
        for r in &subtract(bounds, &holes) {
            let window =
                MaskWindow::new(*r, DIM_ALPHA).map_err(|e| ErrorBody::new(codes::INTERNAL, e))?;
            masks.push(window);
        }

        if !active
            .effect
            .set_pixels(union.x, union.y, union.w, union.h, &bgra)
        {
            return Err(ErrorBody::new(codes::INTERNAL, "提交位图失败"));
        }
        // 调试构建允许关掉截图排除，方便用探针看到真实画面。
        #[cfg(debug_assertions)]
        let excluded = std::env::var_os("SPOTLIGHT_DEBUG_NO_EXCLUDE").is_none();
        #[cfg(not(debug_assertions))]
        let excluded = true;
        active.effect.set_excluded(excluded);

        set_close_hit(close_hit_screen.map(offset));
        let _ = take_dismiss_request();

        active._masks = masks;
        self.active = Some(active);
        self.last_activity = Instant::now();
        self.arm();

        Ok(ShowResult {
            scene: scene.key.clone().unwrap_or_else(|| "s-1".into()),
            timeout_ms: timeout.map(|d| d.as_millis() as u64),
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

/// 数据根目录：默认 `%LOCALAPPDATA%\spotlight`，可用 `SPOTLIGHT_HOME` 覆盖（便携模式）。
fn capture_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("SPOTLIGHT_HOME") {
        return PathBuf::from(home).join("captures");
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("spotlight")
        .join("captures")
}

fn render_active(active: &Active, opacity: f32) -> Option<Vec<u8>> {
    let progress = match active.total {
        Some(total) if total.as_secs_f32() > 0.0 => Some(Progress {
            value: (active.remaining.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0),
            color: ACCENT,
        }),
        _ => None,
    };

    let ctx = Ctx {
        card: active.geom.card.map(|c| Card {
            rect: RectF::from_rect(c),
            radius: CARD_RADIUS,
            bg: CARD_BG,
        }),
        text: match (&active.text, active.geom.text_at) {
            (Some(tb), Some((x, y))) => Some(TextLayer {
                rect: RectF::new(x, y, tb.width as f32, tb.height as f32),
                mask: &tb.alpha,
                mask_width: tb.width,
                mask_height: tb.height,
                color: [255, 255, 255, 255],
            }),
            _ => None,
        },
        progress,
        close: active.geom.close.map(|c| CloseMark {
            rect: RectF::from_rect(c),
            color: [255, 255, 255, 150],
        }),
    };

    let targets: Vec<RectF> = active
        .geom
        .targets
        .iter()
        .map(|r| RectF::from_rect(*r))
        .collect();

    render_frame(
        active.geom.union.w as u32,
        active.geom.union.h as u32,
        &targets,
        &active.effects,
        &ctx,
        opacity,
    )
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
        Method::Clear => {
            if let Err(err) = req.parse_params::<ClearParams>() {
                return Response::err(id, err);
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
                            &Response::err(0, ErrorBody::new(codes::BAD_JSON, err.to_string())),
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
                        &Response::err(id, ErrorBody::new(codes::TIMEOUT, "守护进程处理超时")),
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
    let _guard = match SingleInstance::acquire("Local\\spotlight-daemon") {
        Ok(guard) => guard,
        Err(_) => return 0, // 已经有一个在跑
    };

    let bus = MessageBus::<Job>::new();
    let sender = bus.sender();
    let server = Server::new(&ipc::pipe_name());

    std::thread::spawn(move || {
        loop {
            let mut conn = match server.accept() {
                Ok(conn) => conn,
                Err(_) => break,
            };
            serve(&mut conn, &sender);
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
        }
        if state.should_exit {
            break;
        }
    }
    0
}
