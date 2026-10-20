# Spotlight

**给 AI 用的一支激光笔。**

AI 能看懂截图，也能写出正确的操作步骤，但它**指不了**——用户只能对着
"在开始选项卡里、字体右边那个按钮"这种描述在几十个图标里找。

Spotlight 补上这一环：AI 把截图里量到的矩形传过来，屏幕上其余部分就压暗，
目标被圈出来，旁边一句话说明该怎么点。

```
AI:   capture → 看截图 → 量出矩形 → show
用户: [屏幕压暗，目标被红框圈住，旁边写着"点这个『边框』按钮"]
```

一个单文件可执行程序，约 0.9 MB，零运行时依赖（不需要 .NET / WebView2 / VC++ 运行库）。

## 它能做什么

| 能力 | 说明 |
| --- | --- |
| 截图 + 坐标映射 | AI 在图上量像素，工具负责换算到屏幕物理像素 |
| 高亮 + 文案 | 整屏压暗、目标留亮，配标题 / 正文 / 快捷键提示 |
| 多种视觉效果 | `ring` 描边、`brush` 手绘圈、`glow` 外发光、`arrow` 指向连线、`crosshair` 四角准星 |
| 多目标 + 步骤 | 一次标出好几处，各自带编号角标 |
| 自动消失 | 按文案长度估算停留时间，也可以常驻 |
| 随时关掉 | 卡片关闭按钮 / `Ctrl+Alt+Esc` / `spotlight clear` |

## 快速开始

下载 `spotlight.exe`，放到任意目录，直接运行：

```powershell
# 直接给屏幕坐标
.\spotlight.exe show --rect 760,420,240,70 --text "点这里去掉边框"

# 或者先截图，在图上量坐标（推荐，不用自己算 DPI）
.\spotlight.exe capture --json
.\spotlight.exe show --capture c-1 --rect 500,300,200,60 --text "点这里"
```

第一次调用会自动在后台拉起守护进程（约 80ms），之后的调用只要几毫秒。
守护进程在没有画面的情况下空闲 3 分钟会自动退出。

想在任何目录里直接敲 `spotlight`，把这个 exe 所在目录加进用户 PATH 即可。

## 给 AI 的调用流程

1. `spotlight capture --json` —— 截一张图，返回图片路径和坐标映射
2. 在**图片像素**上量出目标矩形
3. `spotlight show --capture <id> --rect x,y,w,h --text "..."`

坐标必须二选一：`--capture <id>`（推荐）或 `--space screen`。两个都不给会直接报错并
给出下一步提示——宁可不画，也不要在某个 DPI 组合下静默偏 30 像素。

批量准备场景时可以加 `--dry-run` 先把坐标全部验一遍：它返回换算后的屏幕矩形和预计
时长，但不会在屏幕上画任何东西。

复杂场景用 JSON 文件：

```bash
spotlight show --file scene.json
```

```json
{
  "capture": "c-1",
  "targets": [
    { "rect": [500, 300, 200, 60], "step": "1" },
    { "rect": [820, 520, 160, 44], "step": "2" }
  ],
  "title": "两步完成",
  "text": "先点这里，再点下面那处",
  "shortcut": "Ctrl+B",
  "effects": ["ring", "arrow"],
  "timeout": 20
}
```

可直接运行的例子在 [`examples/scene.json`](examples/scene.json)（单步）和
[`examples/tour.json`](examples/tour.json)（多步）。

多步流程用 `steps` 数组：按热键（默认 `Ctrl+Alt+Esc`）跳到下一步，卡片最后一行会写
「下一步：…」，最后一步时热键就是关闭。

点击行为是默认的：**遮罩区域放行点击**（用户还能操作下面的应用），**点中高亮区域就进入下一步**。
点高亮区域的那次点击会被工具收下，再补发一次给下面的应用（受限环境里补发可能无效）。
## 命令

| 命令 | 作用 |
| --- | --- |
| `capture` | 截图并返回坐标映射（默认主屏，`--monitor N` 选屏） |
| `captures` | 列出守护进程当前持有的截图（id / 尺寸 / 多久前） |
| `show` | 显示高亮（`--rect` 可重复，或用 `--file`） |
| `clear` | 立即清除当前画面 |
| `reload` | 让守护进程重新读取配置，不必等它自己退出 |
| `status` | 查看守护进程与当前画面 |
| `doctor` | 自检：屏幕、DPI、配置、守护进程、热键（`--privacy` 列出截图缓存） |
| `--json` | 上述命令都支持机器可读输出 |

常用参数：

```
--rect x,y,w,h     目标矩形（可重复）
--capture <id>     坐标来自哪张截图
--text / --title   正文 / 标题
--step <1/3>       步骤标记（可重复，按序对应 --rect）
--shortcut "Ctrl+B" 快捷键提示
--effect <名字>    ring / brush / glow / arrow / crosshair（可重复）
--timeout <秒|0>   0 表示常驻；缺省按字数估算
--placement <方位> auto / top / bottom / left / right（放不下会自动换边）
--dry-run          只校验坐标与参数、返回换算结果，不真的显示
--key <id>         相同 key 原地替换，不重播入场动画
```

退出码：`0` 成功、`2` 参数错误、`3` 坐标或截图问题、`4` 守护进程不可用、`5` key 不存在。

## 配置

`<数据目录>/config.toml`，Windows 默认在 `%LOCALAPPDATA%\spotlight\`；
设置环境变量 `SPOTLIGHT_HOME` 可以整体挪走（便携部署用）。

```toml
[visual]
theme      = "dark"      # dark | light
accent     = "#FF3B30"   # #RRGGBB 或 #RRGGBBAA
dim        = 0.55        # 遮罩不透明度 0.05–0.90
ring_width = 3
radius     = 10

[callout]
font_px        = 14
max_text_width = 336

[effects]
default = ["ring"]       # 没传 --effect 时用这组

[pause]
cap_seconds = 60         # 鼠标停在文案上最多暂停多久

[hotkey]
dismiss = "Ctrl+Alt+Esc" # 关闭当前画面的全局热键
```

写错了不会启动失败：**只有出错的那一项**回退默认值，`spotlight doctor` 里能看到说明。
改完配置跑一次 `spotlight reload` 就生效，不用等守护进程自己退出。

## 已知限制

- 目前只支持 **Windows**（macOS 在路线图上，需要一台 Mac 才能做）
- 全屏独占应用（部分游戏、播放器）会盖住高亮
- 二进制没有代码签名，首次运行可能出现 SmartScreen 提示
- **不做鼠标键盘模拟**——它只负责"指"，不负责"点"

## 开发

需要 Rust **1.88+**（MSRV 已用该版本实测通过），Windows 下直接构建：

```
cargo test                   # 覆盖 core / render / platform / CLI 四层
cargo build --release        # 产物在 target/release/spotlight.exe
```

release 配置是 `opt-level = "z"` + fat LTO + `panic = "abort"` + strip，
产物约 0.9 MB。要再压体积可以装 `upx` 自行处理（不写进构建，免得杀软误报）。

设计文档在 [`ai-docs/design-docs`](ai-docs/design-docs/README.md)：为什么这么设计、
坐标契约怎么定、安全边界在哪，以及**哪些功能明确决定不做**。
变更记录在 [`CHANGELOG.md`](CHANGELOG.md)。

## License

MIT
