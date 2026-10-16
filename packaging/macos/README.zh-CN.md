# Ikura macOS Apple Silicon 测试版

适用于 M 系列 Mac。最低系统版本见本安装包的 RELEASE_NOTES.md；构建脚本会根据实际内置依赖计算该版本。

## 安装与启动

1. 将 Ikura.app 拖到“应用程序”或其他可写目录，勿在压缩包内运行。
2. 准备自己的 game_data（见下方结构）。此下载仅包含解释器及运行依赖，不包含游戏素材或汉化文本。
3. 双击 Ikura.app，选择 game_data。以后会记住这个目录。
4. 若目录中有 zh-CN.json，首次启动可选择中文或日文。
5. 按住 Option 打开应用，可以重新选择目录和语言。

本测试包使用本机临时签名，未作 Apple Developer ID 公证。若 macOS 阻止首次启动，请在“系统设置 → 隐私与安全性”中查看并允许这个应用；详情参见 https://support.apple.com/102445 。

## 数据目录

```text
game_data/
  scripts.json        原版 ISF 的解析结果
  PNG-images/         解码后的图片（GGD、GGD2 等子目录）
  OGG-audio/          音乐、音效和语音（WMSC、SE、VOICE1、VOICE2）
  DATA                原游戏的 SM2MPX DATA 文件（光标和转场遮罩）
  DOLOGO.MPG          原版片头视频（也可放在 game_data 的上级目录）
  zh-CN.json          可选：从自己的汉化补丁导出的中文文本包
```

已有开发用 game_data 的用户：把项目 `data/scripts.json` 复制到这个目录；
中文用户同时复制 `data/zh-CN.json`。把原游戏的 `DATA` 和 `DOLOGO.MPG` 放到上图位置。
资源解码和文本导出步骤见源码仓库 README；测试版需要已经解包、解码的资源，尚不能直接打开原版 Windows 安装目录。

## 控制

- 左键 / 空格 / Enter：补全文字，再按推进剧情。
- 方向键 / Enter：选择菜单和选项。
- Ctrl：快进；A：自动阅读；H / 右键：隐藏或显示对话框。
- B：历史记录；滚轮或拖动滚动条浏览，点击带语音标记的台词重播。
- F5 / F9：快速存档 / 读档；F6 / F8：打开存档 / 读档菜单。
- F10 或右上角按钮：设置。Mac 功能键可能需要配合 Fn。
- Esc：主画面退出；菜单中返回；片头播放时跳过片头。

存档和设置位于 `~/Library/Application Support/Ikura/saves/`。
启动日志为 `~/Library/Application Support/Ikura/last-launch.log`。
原来开发环境的 `game_data/rust-saves` 和 `rust-saves-zh` 不会被自动迁移；需要时可将其 `.iks` 文件复制到新存档目录（先确认游戏版本一致）。

## 已知限制

这是预发布测试版：部分特殊转场仍为近似效果，尚未对所有分支做实时音画验收。
新存档支持历史记录与音频时间恢复，但不保证音频逐采样精确同步。
SDL 和 FFmpeg 随包提供，不需要安装 Rust 或 Homebrew。

依赖来源、构建系统要求及提交版本见 RELEASE_NOTES.md 和 THIRD_PARTY.json。
