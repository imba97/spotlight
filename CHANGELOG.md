# Changelog

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [0.1.0] — 2026-10-19

第一个可用版本：Windows 上的完整闭环——AI 截图、量坐标、把目标指给用户看。

### 命令

- `capture`：截图并返回坐标映射，`--monitor N` 选屏，长边默认缩到 1280
- `captures`：列出守护进程当前持有的截图（id / 尺寸 / 多久前）
- `show`：高亮目标，`--rect` 可重复，支持 `--text` / `--title` / `--step` / `--shortcut`
- `show --file <scene.json>`：从文件读多目标场景，命令行参数可覆盖文件里的字段
- `show --dry-run`：只校验坐标和参数，返回换算后的屏幕矩形，不创建任何窗口
- `clear` / `status` / `doctor` / `reload` / `version`
- 所有命令支持 `--json`
- `show --file` 支持 `steps` 多步流程：按热键（默认 `Ctrl+Alt+Esc`）跳到下一步，卡片末行显示下一步提示
- `--click-through`：默认遮罩拦截点击（遮罩下的东西点不动），加了才放行
- `--click-advance`：点中高亮区域就进入下一步（那次点击会被收下，再由工具补发一次）

### 视觉

- 整屏压暗，目标开洞留亮；遮罩按矩形切分，不为全屏建位图
- 遮罩洞按圆角补角：环的圆角外侧不再露出亮角
- 管道名带构建指纹：重新编译后不会连到旧守护进程
- 五种效果：`ring` 圆角描边、`brush` 手绘圈、`glow` 外发光、`arrow` 指向连线、`crosshair` 四角准星
- 卡片：标题 / 正文 / 快捷键键帽 / 倒计时进度条 / 关闭按钮
- show 响应带 debug：卡片实际矩形、选择的方位、所在屏、DPI 缩放
- 文案超出高度上限时会截断，同时在响应里返回 `warnings`（CLI 打到 stderr），不默默切掉
- 深浅两套主题，所有尺寸按显示器 DPI 缩放
- 120ms 淡入；相同 `--key` 的更新不重播动画

### 交互

- 鼠标穿透，只有关闭按钮可点
- `Ctrl+Alt+Esc`（可配置）关闭，`--timeout` 自动消失，鼠标停在文案上暂停计时
- 显示器配置变化时立即关闭画面对应的旧坐标
- 超长文案按宽度自动折行，超过 512px 高会被裁剪

### 工程

- Rust workspace：`core`（纯逻辑、无 unsafe）/ `render`（tiny-skia）/ `platform`（Win32）/ `cli`
- 常驻守护进程按需拉起，空闲 3 分钟退出；IPC 走命名管道，每用户独立
- 单文件可执行，release 约 0.9 MB，无运行时依赖
- 截图缓存目录在守护进程启动时清空，不跨会话留存
- 112 个测试（含 CLI 集成测试与随机布局不变量），`clippy -D warnings` 干净

### 已知限制

- 仅支持 Windows；macOS 需要一台 Mac 才能做
- 全屏独占应用会盖住高亮
- 二进制无代码签名，首次运行可能出现 SmartScreen 提示
- 不做鼠标键盘模拟
