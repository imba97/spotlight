//! 守护进程：持有所有 overlay 窗口，串行处理 IPC 请求。
//!
//! 所有窗口操作都在主线程；IPC 线程只负责收包和回包。

use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::json;
use spotlight_core::geom::{Rect, RectF, ScreenRect};
use spotlight_core::layout::{Size, place};
use spotlight_core::mask::subtract;
use spotlight_core::protocol::{
    ClearParams, ErrorBody, Method, Request, Response, SceneSummary, ShowParams, ShowResult,
    StatusResult, codes,
};
use spotlight_core::scene::{Callout, Scene};
use spotlight_core::timing::{Timeout, resolve};
use spotlight_platform::win::effect::EffectWindow;
use spotlight_platform::win::ipc::{self, Connection, Server};
use spotlight_platform::win::message::{self, Event, MessageBus};
use spotlight_platform::win::process::SingleInstance;
use spotlight_platform::win::text::render_text;
use spotlight_platform::win::{MaskWindow, screen_size};
use spotlight_render::{Card, Frame, Ring, TextLayer, render_frame};

const DIM_ALPHA: u8 = 140;
/// 洞相对目标的 padding。
const PAD: i32 = 6;
const RING_STROKE: f32 = 3.0;
const RING_RADIUS: f32 = 10.0;
const ACCENT: [u8; 4] = [255, 59, 48, 255];
const CARD_BG: [u8; 4] = [27, 27, 31, 240];
const CARD_RADIUS: f32 = 10.0;
const CARD_PADDING: i32 = 12;
const CARD_GAP: i32 = 12;
const FONT_PX: i32 = 14;
const MAX_TEXT_WIDTH: u32 = 336;

const TICK_ACTIVE_MS: u32 = 100;
const TICK_IDLE_MS: u32 = 5000;
const IDLE_EXIT: Duration = Duration::from_secs(180);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

struct Job {
    req: Request,
    reply: mpsc::SyncSender<Response>,
}

struct Active {
    key: Option<String>,
    has_text: bool,
    deadline: Option<Instant>,
    /// 窗口随这个结构一起销毁。
    _masks: Vec<MaskWindow>,
    _effect: Option<EffectWindow>,
}

struct State {
    started: Instant,
    last_activity: Instant,
    active: Option<Active>,
    timer_ms: u32,
    should_exit: bool,
}

impl State {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            last_activity: Instant::now(),
            active: None,
            timer_ms: 0,
            should_exit: false,
        }
    }

    fn arm(&mut self) {
        let want = if self.active.is_some() {
            TICK_ACTIVE_MS
        } else {
            TICK_IDLE_MS
        };
        if self.timer_ms != want {
            message::stop_timer();
            message::start_timer(want);
            self.timer_ms = want;
        }
    }

    fn tick(&mut self) {
        let expired = self
            .active
            .as_ref()
            .and_then(|a| a.deadline)
            .is_some_and(|deadline| Instant::now() >= deadline);
        if expired {
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
        self.arm();
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
                remaining_ms: a
                    .deadline
                    .map(|d| d.saturating_duration_since(Instant::now()).as_millis() as u64),
            }),
        }
    }

    fn show(&mut self, p: ShowParams) -> Result<ShowResult, ErrorBody> {
        let rect = Rect::new(p.rect[0], p.rect[1], p.rect[2], p.rect[3]);
        if rect.is_empty() {
            return Err(
                ErrorBody::new(codes::BAD_PARAMS, "矩形为空").with_hint("w 和 h 必须大于 0")
            );
        }

        let (screen_w, screen_h) = screen_size();
        let bounds = Rect::new(0, 0, screen_w, screen_h);
        if !bounds.contains_rect(rect) {
            return Err(
                ErrorBody::new(codes::RECT_OUT_OF_BOUNDS, "矩形超出屏幕范围").with_hint(format!(
                    "当前屏幕是 {screen_w}×{screen_h}，请重新截图后按图像像素给坐标"
                )),
            );
        }

        let mut scene = Scene::new(ScreenRect::from_rect(rect));
        scene.key = p.key.clone();
        scene.timeout = match p.timeout {
            None => Timeout::Auto,
            Some(secs) if secs <= 0.0 => Timeout::Sticky,
            Some(secs) => Timeout::Seconds(secs),
        };
        if let Some(text) = p.text.clone() {
            scene.callout = Some(Callout {
                title: p.title.clone(),
                text,
            });
        }
        if let Some(step) = p.step.clone() {
            if let Some(target) = scene.targets.first_mut() {
                target.step = Some(step);
            }
        }
        scene
            .validate()
            .map_err(|e| ErrorBody::new(codes::BAD_PARAMS, e.to_string()))?;

        // 先算几何，再建窗口——任何一步失败都不会留下半截画面。
        let ring_rect = rect.inflate(PAD + RING_STROKE as i32);
        let text_bitmap = scene
            .callout
            .as_ref()
            .and_then(|c| render_text(&c.text, MAX_TEXT_WIDTH, FONT_PX));
        let card_rect = text_bitmap.as_ref().map(|tb| {
            place(
                rect,
                Size::new(
                    tb.width as i32 + CARD_PADDING * 2,
                    tb.height as i32 + CARD_PADDING * 2,
                ),
                bounds,
                CARD_GAP,
            )
            .rect
        });

        let mut union = ring_rect;
        if let Some(card) = card_rect {
            union = union.union(card);
        }

        let offset = |r: Rect| {
            RectF::new(
                (r.x - union.x) as f32,
                (r.y - union.y) as f32,
                r.w as f32,
                r.h as f32,
            )
        };

        let frame = Frame {
            width: union.w as u32,
            height: union.h as u32,
            ring: Some(Ring {
                rect: offset(ring_rect),
                stroke: RING_STROKE,
                radius: RING_RADIUS,
                color: ACCENT,
            }),
            card: card_rect.map(|c| Card {
                rect: offset(c),
                radius: CARD_RADIUS,
                bg: CARD_BG,
            }),
            text: match (&text_bitmap, card_rect) {
                (Some(tb), Some(card)) => Some(TextLayer {
                    rect: offset(Rect::new(
                        card.x + CARD_PADDING,
                        card.y + CARD_PADDING,
                        tb.width as i32,
                        tb.height as i32,
                    )),
                    mask: &tb.alpha,
                    mask_width: tb.width,
                    mask_height: tb.height,
                    color: [255, 255, 255, 255],
                }),
                _ => None,
            },
        };

        let Some(bgra) = render_frame(&frame) else {
            return Err(ErrorBody::new(codes::INTERNAL, "渲染失败"));
        };

        self.clear_all();

        let holes = [rect.inflate(PAD)];
        let mut masks = Vec::new();
        for r in &subtract(bounds, &holes) {
            let window =
                MaskWindow::new(*r, DIM_ALPHA).map_err(|e| ErrorBody::new(codes::INTERNAL, e))?;
            masks.push(window);
        }

        let effect = EffectWindow::new(union).map_err(|e| ErrorBody::new(codes::INTERNAL, e))?;
        if !effect.set_pixels(union.x, union.y, union.w, union.h, &bgra) {
            return Err(ErrorBody::new(codes::INTERNAL, "提交位图失败"));
        }
        // 调试构建允许关掉截图排除，方便用探针看到真实画面。
        #[cfg(debug_assertions)]
        let excluded = std::env::var_os("SPOTLIGHT_DEBUG_NO_EXCLUDE").is_none();
        #[cfg(not(debug_assertions))]
        let excluded = true;
        effect.set_excluded(excluded);

        let timeout = resolve(scene.timeout, scene.text());
        let deadline = timeout.map(|d| Instant::now() + d);
        let has_text = scene.callout.is_some();

        self.active = Some(Active {
            key: scene.key.clone(),
            has_text,
            deadline,
            _masks: masks,
            _effect: Some(effect),
        });
        self.last_activity = Instant::now();
        self.arm();

        Ok(ShowResult {
            scene: scene.key.clone().unwrap_or_else(|| "s-1".into()),
            timeout_ms: timeout.map(|d| d.as_millis() as u64),
        })
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
