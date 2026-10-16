# 家族計画 ～追憶～ 跨平台移植

这是一个用 Rust 编写的《家族計画 ～追憶～》非官方跨平台运行器。项目通过解析游戏脚本并模拟 Ikura 引擎的行为，在 SDL 窗口中运行剧情、显示画面、播放音乐和语音。

本项目仍在开发中。它不是原版游戏，也不包含游戏本体、图片、音乐、语音、视频或汉化补丁。运行时需要用户自行准备合法取得的游戏文件，并按下文导出和整理资源。

## 当前状态

- **macOS Apple Silicon：** 可构建可双击启动的测试版，支持选择游戏数据目录。GitHub Actions 已配置手动构建 Release 草稿的流程。
- **Android：** 有 arm64-v8a 调试构建流程，可将准备好的资源打包进 APK。
- **Windows 和 Linux：** Rust 与 SDL 依赖按平台配置；目前主要开发和验证集中在 macOS，尚未提供同等成熟的用户安装包。
- **游戏进度：** 已覆盖标题、主要剧情、对话、选项、存档、设置、历史记录、音乐、语音及多种转场。已验证若干完整剧情路线，但并未穷举每个选项，也未对全游戏进行逐场景的原版音画对比。

部分特殊转场仍为近似效果；多人对话的文字出现时序、字体光栅效果和部分引擎指令也可能与原版不同。详见“已知限制”。

## macOS Apple Silicon 测试版

测试版面向 M 系列 Mac。它包含运行器及 SDL、FFmpeg 等运行依赖，不需要安装 Rust 或 Homebrew。测试包采用临时签名，尚未经过 Apple Developer ID 公证；首次打开时若 macOS 提示无法验证，请按[苹果的说明](https://support.apple.com/102445)在“系统设置 → 隐私与安全性”中允许打开。

### 准备游戏数据

测试包需要已经解码、导出的资源目录。它不能直接读取原版 Windows 安装目录。目录大致如下：

```text
game_data/
├── scripts.json       # 从 ISF 脚本导出的 JSON
├── PNG-images/        # 解码后的图片资源，如 GGD、GGD2
├── OGG-audio/         # 音乐、音效和语音
├── DATA               # 原版 DATA 资源柜，用于光标和部分转场
├── DOLOGO.MPG         # 可选的原版开场视频
└── zh-CN.json         # 可选的中文文本包
```

如果已经按本项目的开发流程准备过资源，可将项目 `data/scripts.json` 复制到 `game_data/`；中文模式还需将 `data/zh-CN.json` 一并复制。开场视频可以放在 `game_data` 或其上级目录。图片和音频目录名称应与示例一致。

汉化文本包需要从用户自己的补丁生成。仓库不提供原补丁或生成后的文本。现有导出脚本对当前补丁的台词匹配率约为 95.5%，少量未匹配内容仍会显示日文。

### 安装并启动

1. Release 发布后，从 GitHub Releases 下载 `.dmg` 或 `.zip`，将 `Ikura.app` 放入“应用程序”或其他可写目录。
2. 双击启动，在目录选择窗口中选取准备好的 `game_data`。
3. 若目录内有 `zh-CN.json`，首次启动时可选择中文或日文。应用会记住目录和语言；按住 Option 启动可重新选择。

存档、设置和启动日志位于：

```text
~/Library/Application Support/Ikura/
```

安装步骤、数据目录细节和已知问题也收录在[macOS 测试版说明](packaging/macos/README.zh-CN.md)。

## 操作方式

| 按键 | 功能 |
| --- | --- |
| 鼠标左键、空格、Enter | 补全文字；文字显示完后再次按下以继续 |
| 方向键、Enter | 浏览并确认标题菜单或剧情选项 |
| Ctrl | 快进；遇到选项时停止 |
| A | 切换自动阅读 |
| H 或鼠标右键 | 隐藏或显示对话框 |
| B | 打开历史记录；点击带语音标记的台词可重播 |
| F5、F9 | 快速存档、快速读档 |
| F6、F8 | 打开存档、读档菜单 |
| F10 | 打开设置；也可点击窗口右上角的设置按钮 |
| Esc | 返回或退出；播放片头时跳过片头 |

存档菜单提供 12 个手动档位以及快速档、自动档。设置包括音乐、音效和语音音量，文字速度、自动阅读等待、自动存档和全屏。Mac 键盘可能需要同时按 Fn 才能触发 F5–F10。

Android 版本提供触屏快捷菜单和虚拟按键面板。标题画面点击右上角“菜单”或按手机返回键可打开快捷菜单；剧情会在菜单打开时暂停。面板支持存读档、自动阅读、快进、历史记录、设置和常用按键。

## 从源码运行

### macOS 开发运行

需要 Rust、Python 3、SDL2、SDL2_mixer、FFmpeg、FFprobe，以及已导出的游戏数据。安装依赖：

```sh
brew install pkg-config sdl2 sdl2_mixer ffmpeg
```

先使用现有 ISF 解析工具导出脚本，再启动运行器：

```sh
python3 tools/export_isf.py \
  --parser /path/to/inspect_isf.py \
  --decoded /path/to/ISF-decoded \
  --output data/scripts.json

cargo run -- /path/to/game_data
```

若片头视频不在 `game_data` 或其上级目录，可设置 `IKURA_VIDEO_DIR` 指向原游戏目录。开发运行、中文补丁导出及 macOS 打包命令见下方资料链接。

### Android 调试构建

Android 工程位于 `android/`，当前构建目标为 arm64-v8a。需要 Android SDK、NDK 27.2.12479018、CMake 3.22.1、JDK 21 和 Rust 的 `aarch64-linux-android` target。准备好原版游戏目录和解码数据后运行：

```sh
tools/stage_android_assets.sh /path/to/original-game /path/to/game_data
cd android
./gradlew assembleDebug
```

APK 位于 `android/app/build/outputs/apk/debug/app-debug.apk`。Android 首次启动会将资源解压到应用私有目录；目前跳过开场视频，不使用外部 FFmpeg 解码。

## 已知限制

- 尚未完整复刻所有脚本指令和特殊画面效果。`GP 24/25/28/29` 等转场以及摇晃参数仍需和原版逐项对照；部分 GGE 遮罩效果也只覆盖已观察到的参数组合。
- 多人对话可显示，但文字同步出现时序和字体渲染与原版有差异。
- 存档使用本项目自己的格式，不能与原版 Windows 游戏互通。
- 已测试若干完整剧情路线，但其他选项组合、全游戏实时音画同步和每个界面状态仍需更多验证。
- Android 目前是调试构建流程；Windows、Linux 尚无成熟的发行包。

## 项目资料

- [macOS Apple Silicon 测试版说明](packaging/macos/README.zh-CN.md)：安装、资源准备、操作及故障排查。
- [macOS Release 工作流](.github/workflows/macos-release.yml)：手动构建测试版草稿的 GitHub Actions 配置。
- `tools/export_isf.py`：调用现有 Python ISF 解析器导出脚本 JSON。
- `tools/export_chinese_patch.py`：从本机已有汉化补丁生成中文文本包。
- `tools/package_macos.py`：构建并打包 macOS Apple Silicon 应用。

## 版权与资源

本项目是粉丝制作的兼容性移植，与原游戏权利人无关。游戏脚本、图片、音乐、语音、视频和汉化补丁均不随源码或测试包分发。请只使用自己有权使用的游戏文件，并遵守原游戏及补丁各自的许可和使用条款。
